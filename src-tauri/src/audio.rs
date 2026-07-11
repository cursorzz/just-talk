use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

pub struct AudioCapture {
    _stream: cpal::Stream,
    active: Arc<AtomicBool>,
}

impl AudioCapture {
    pub fn start(sender: mpsc::UnboundedSender<Vec<i16>>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("没有找到可用麦克风")?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("读取麦克风格式失败：{e}"))?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let active = Arc::new(AtomicBool::new(true));
        let error_callback = |error| eprintln!("audio stream error: {error}");

        let stream = match supported.sample_format() {
            cpal::SampleFormat::I16 => {
                let active = active.clone();
                let mut converter = AudioConverter::new(channels, sample_rate);
                device.build_input_stream(
                    supported.config(),
                    move |data: &[i16], _| {
                        send_converted(data.iter().copied(), &mut converter, &sender, &active)
                    },
                    error_callback,
                    None,
                )
            }
            cpal::SampleFormat::F32 => {
                let active = active.clone();
                let mut converter = AudioConverter::new(channels, sample_rate);
                device.build_input_stream(
                    supported.config(),
                    move |data: &[f32], _| {
                        send_converted(
                            data.iter()
                                .map(|v| (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16),
                            &mut converter,
                            &sender,
                            &active,
                        )
                    },
                    error_callback,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let active = active.clone();
                let mut converter = AudioConverter::new(channels, sample_rate);
                device.build_input_stream(
                    supported.config(),
                    move |data: &[u16], _| {
                        send_converted(
                            data.iter().map(|v| (*v as i32 - 32768) as i16),
                            &mut converter,
                            &sender,
                            &active,
                        )
                    },
                    error_callback,
                    None,
                )
            }
            format => return Err(format!("不支持的麦克风采样格式：{format:?}")),
        }
        .map_err(|e| format!("启动麦克风失败：{e}"))?;

        stream.play().map_err(|e| format!("开始录音失败：{e}"))?;
        Ok(Self {
            _stream: stream,
            active,
        })
    }

    pub fn stop(&self) {
        self.active.store(false, Ordering::Relaxed);
    }
}

fn send_converted<I: Iterator<Item = i16>>(
    samples: I,
    converter: &mut AudioConverter,
    sender: &mpsc::UnboundedSender<Vec<i16>>,
    active: &AtomicBool,
) {
    if !active.load(Ordering::Relaxed) {
        return;
    }
    let output = converter.process(samples.collect());
    if !output.is_empty() {
        let _ = sender.send(output);
    }
}

struct AudioConverter {
    channels: usize,
    resampler: StreamingResampler,
}

impl AudioConverter {
    fn new(channels: usize, input_rate: u32) -> Self {
        Self {
            channels,
            resampler: StreamingResampler::new(input_rate, 16_000),
        }
    }

    fn process(&mut self, interleaved: Vec<i16>) -> Vec<i16> {
        if self.channels == 0 {
            return Vec::new();
        }
        let mono = interleaved
            .chunks(self.channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|v| *v as i32).sum();
                (sum / frame.len() as i32) as i16
            })
            .collect::<Vec<_>>();
        self.resampler.process(&mono)
    }
}

struct StreamingResampler {
    input_rate: u32,
    output_rate: u32,
    step: f64,
    position: f64,
    tail: Vec<i16>,
}

impl StreamingResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
            step: input_rate as f64 / output_rate as f64,
            position: 0.0,
            tail: Vec::new(),
        }
    }

    fn process(&mut self, input: &[i16]) -> Vec<i16> {
        if input.is_empty() {
            return Vec::new();
        }
        if self.input_rate == self.output_rate {
            return input.to_vec();
        }
        let mut merged = std::mem::take(&mut self.tail);
        merged.extend_from_slice(input);
        let mut output = Vec::new();
        loop {
            let left = self.position.floor() as usize;
            let right = left + 1;
            if right >= merged.len() {
                break;
            }
            let fraction = self.position - left as f64;
            let sample = merged[left] as f64 * (1.0 - fraction) + merged[right] as f64 * fraction;
            output.push(sample.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
            self.position += self.step;
        }
        let base = self.position.floor() as usize;
        let keep_from = base.saturating_sub(1).min(merged.len());
        self.tail = merged[keep_from..].to_vec();
        self.position -= keep_from as f64;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::StreamingResampler;

    #[test]
    fn chunked_resampling_matches_single_stream() {
        let input = (0..4_410)
            .map(|v| ((v % 200) - 100) as i16 * 100)
            .collect::<Vec<_>>();
        let mut whole = StreamingResampler::new(44_100, 16_000);
        let expected = whole.process(&input);
        let mut chunked = StreamingResampler::new(44_100, 16_000);
        let actual = input
            .chunks(137)
            .flat_map(|chunk| chunked.process(chunk))
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .zip(expected.iter())
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
            "chunk boundaries must not introduce an audible discontinuity"
        );
    }
}
