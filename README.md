# JustTalk

`just-talk-slim` 是 `just-talk` 的轻量桌面重写版本。应用使用 Rust + Tauri，界面由系统 WebView 渲染，不再捆绑 Python、Qt 和 Chromium。

## 当前能力

- 豆包 SAUC 语音识别协议
- 16 kHz 单声道 PCM 实时录音与 200 ms 分片发送
- 自由模式快捷键：按下录音，松开停止并识别（默认）
- 常规模式快捷键：按一次开始，再按一次停止
- 连接前录音缓冲和最后一帧处理
- 识别结果自动复制、可选自动粘贴
- 系统托盘和关闭到托盘
- 本地 JSON 配置
- 输入方式和高级设置即时保存
- 录音时自动暂停系统媒体会话，退出录音后恢复
- macOS 麦克风和辅助功能强制权限门禁

完整产品行为与跨平台边界见 [产品与实现决策](docs/PRODUCT_DECISIONS.md)。

## 火山引擎语音服务

- 所需模块：豆包流式语音识别模型 2.0（小时版）。
- Resource ID：`volc.seedasr.sauc.duration`。
- API 模式：通过 WebSocket 调用 `bigmodel_nostream` 流式输入接口，地址为 `wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream`。

开通步骤：

1. 进入[火山引擎豆包语音控制台](https://console.volcengine.com/speech/new/)，开通“豆包流式语音识别模型 2.0（小时版）”。
2. 在控制台获取 App ID 和 Access Token。具体接口参数参见[大模型流式语音识别 API 文档](https://www.volcengine.com/docs/6561/1354869?lang=zh)。
3. 在 JustTalk 的“语音服务”中填写凭据，点击“测试连接”，测试成功后保存服务设置。

录音数据仅保存在内存中，以约 200 ms 的 PCM 分片直接发送至火山引擎 API；JustTalk 不在本地写入或保留录音文件。停止、取消或识别异常后，剩余的内存音频缓冲会被释放。

## 开发

需要 Node.js 20+、Rust stable，以及对应平台的 Tauri 系统依赖。

```bash
npm install
npm run tauri dev
```

也可以统一使用 Makefile：

```bash
make dev       # 启动开发版本
make test      # 前端构建和 Rust 单元测试
make check     # 编译与 Clippy 严格检查
make build     # 生成 Release .app 和 .dmg
make verify    # 校验签名结构、DMG 和 SHA-256
make clean     # 清理全部构建产物
```

推送与应用版本一致的 `v*` 标签（例如版本 `0.1.2` 对应 `v0.1.2`），会生成 macOS ARM64 DMG、Windows x64 NSIS、Linux x64 AppImage/DEB，并在三个构建全部成功后自动创建 GitHub Release。Release 同时包含所有安装包和 `SHA256SUMS.txt`。

也可以在 GitHub Actions 中手动运行 `Build and release desktop installers`，并填写与 `package.json` 版本一致的发布标签。重复运行同一标签时会更新已有 Release 的安装包。

### macOS 兼容性

- 最低支持 macOS 12.0 Monterey。
- 当前发布产物仅支持 Apple Silicon（ARM64），暂不提供 Intel Mac 安装包。
- 录音、语音识别、快捷键和自动粘贴是稳定主流程。
- “录音时暂停媒体”依赖 Apple 私有的 MediaRemote 接口。macOS 15.4 及以上通过 JXA 获取媒体状态，但该功能仍可能随系统升级或播放器实现变化而失效；媒体控制失败不会影响录音和识别。
- 发布前重点验证 macOS 12、13、14、15.3 和 15.4 及以上版本。

`make build` 会先让 Tauri 只生成 `.app`，然后对完整应用做 ad-hoc 签名，最后通过 `hdiutil` 创建 DMG。这避免了 Tauri 默认 DMG 脚本偶发无法卸载临时映像的问题。产物位于：

```text
src-tauri/target/release/bundle/macos/JustTalk.app
src-tauri/target/release/bundle/dmg/JustTalk_<版本>_<架构>.dmg
```

仅检查前端和 Rust：

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

如果本机 `~/.cargo/config` 仍指向失效的 crates.io 镜像，可临时使用：

```bash
cargo --config 'source.tuna.registry="sparse+https://mirrors.sjtug.sjtu.edu.cn/crates.io-index/"' check --manifest-path src-tauri/Cargo.toml
```

## macOS 权限

应用要求以下权限全部开启后才注册快捷键和开放主功能：

1. 麦克风：录制语音。
2. 辅助功能：通过 Quartz 将识别结果粘贴到当前应用。

全局快捷键通过 macOS 的系统快捷键注册机制实现，不读取用户的键盘输入，因此不要求“输入监控”权限。

应用窗口重新获得焦点时会自动复检。为了避免系统把升级版本视为新应用，正式发布必须保持 `com.justtalk.slim` bundle identifier 和签名身份稳定。
