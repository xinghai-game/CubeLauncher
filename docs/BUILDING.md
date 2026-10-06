# 构建说明

## 依赖

| 平台 | 需要 |
| --- | --- |
| 通用 | Rust 1.80+、Node.js 20+、pnpm 9+ |
| Linux | `libwebkit2gtk-4.1-dev`、`build-essential`、`libssl-dev`、`libayatana-appindicator3-dev`、`librsvg2-dev`、`libxdo-dev` |
| Windows | Microsoft C++ 生成工具、WebView2 运行时（Windows 11 自带） |
| macOS | Xcode 命令行工具（使用系统 WKWebView） |

Linux 上打包 AppImage/DEB 时，Tauri 会下载 `linuxdeploy` 等工具；如果所在网络无法访问
GitHub，可以只构建可执行文件（`cargo build --release -p cube-launcher`），
或为 Tauri 配置代理后重试。

## 常用命令

```bash
pnpm install                 # 安装前端依赖
pnpm tauri dev               # 开发模式，前端热更新
pnpm tauri build             # 生成安装包（AppImage/DEB/NSIS/DMG）
cargo build --release -p cube-launcher   # 只生成可执行文件
cargo test -p launcher-core  # 核心单元测试
cargo clippy --all-targets   # 静态检查
```

前端产物必须存在，`tauri build` 会依次执行 `pnpm build`（见 `tauri.conf.json`
的 `beforeBuildCommand`）。

## 命令行演练

`crates/launcher-core/examples/smoke.rs` 与界面共用同一套核心代码，可在没有图形环境时
验证安装与启动：

```bash
cargo run -p launcher-core --example smoke -- plan    1.20.1
cargo run -p launcher-core --example smoke -- versions forge 1.20.1
cargo run -p launcher-core --example smoke -- install /tmp/cube alice 1.20.1 fabric
cargo run -p launcher-core --example smoke -- verify  /tmp/cube alice --offline
cargo run -p launcher-core --example smoke -- preview /tmp/cube alice
cargo run -p launcher-core --example smoke -- launch  /tmp/cube alice
cargo run -p launcher-core --example smoke -- mods    /tmp/cube alice
cargo run -p launcher-core --example smoke -- java    17 /tmp/cube
```

可选参数：`--offline` 启用严格离线模式，`--mirror <url>` 指定下载镜像，
`--java-mirror <url>` 指定 Adoptium 镜像（例如
`https://mirrors.tuna.tsinghua.edu.cn/Adoptium`，路径形如
`/主版本/类型/架构/系统/文件名`，SHA-256 仍按官方接口校验）。

## 数据目录

默认位置：

- Linux：`~/.local/share/CubeLauncher`
- Windows：`%APPDATA%\CubeLauncher`
- macOS：`~/Library/Application Support/CubeLauncher`

可在“启动器设置”中修改。删除该目录等于恢复出厂状态；实例存档位于
`instances/<实例ID>/.minecraft/saves`。

## 打包说明

- Windows：NSIS 安装包；若需要静默安装或自定义安装路径，可在 `tauri.conf.json`
  的 `bundle.windows.nsis` 中配置。
- macOS：DMG；对外分发需要开发者证书与公证（`bundle.macOS.signingIdentity`）。
- Linux：AppImage 与 DEB；AppImage 对系统 WebKitGTK 版本有要求，建议在
  Ubuntu 22.04 或更新版本上构建。

## 常见问题

**启动器能打开但没有内容**
前端资源缺失或挂载失败。确认 `dist/` 已生成，并在 `tauri build` 前执行一次
`pnpm build`；开发模式下确认 `pnpm dev` 正在 1420 端口运行。

**游戏提示找不到主类**
版本元数据缺少 classpath。1.13 之前的版本没有 `arguments.jvm`，启动器会自动补
`-cp`；如果手工修改过实例的 JVM 参数，请确认没有破坏 classpath 相关参数。

**Forge/NeoForge 安装报“安装清单引用了未知变量”**
安装器格式超出当前实现范围。请附上日志中的处理器名称与变量名。

**下载很慢或 TLS 证书错误**
某些网络会拦截 `maven.minecraftforge.net`、`maven.fabricmc.net` 等主机的证书。
可在设置中填写镜像地址（如 `https://bmclapi2.bangbang93.com`），
或使用 `--mirror` 参数；校验仍然使用官方元数据中的哈希值。
