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

推送 `v*` 标签或在 GitHub Actions 中手动运行 `Build desktop installers`，可生成 macOS ARM64 DMG、Windows x64 NSIS，以及 Linux x64 AppImage/DEB。

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
