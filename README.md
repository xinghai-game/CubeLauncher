# CubeLauncher

一个简洁、轻量的 Minecraft Java 版桌面启动器：支持 Microsoft 正版登录与离线身份、实例隔离、可控性强。

界面用 Svelte 5 编写、由系统 WebView 承载，所有安装与启动逻辑在独立的 Rust 核心里，
因此下载、校验、Java 管理和进程控制都能单独测试，界面也能随时替换。

```
┌──────────────────────────────┐        ┌────────────────────────────────────┐
│ Svelte 5 + TypeScript 界面   │  IPC   │ launcher-core (Rust)               │
│ 仅负责展示与交互             │ ─────▶ │ 元数据 解析 · 下载 校验 · Java 管理 │
└──────────────────────────────┘  事件  │ 加载器 安装 · 启动 参数 · 进程 日志 │
                                       └────────────────────────────────────┘
```

## 功能

| 能力 | 说明 |
| --- | --- |
| 动态版本目录 | 读取 Mojang `version_manifest_v2`，按 HMCL 的方式分为正式版、快照、远古 Beta/Alpha 与愚人节版；支持按版本号搜索与手动刷新，并标明列表来自官方源、镜像还是本地缓存 |
| 分级界面 | 版本目录 → 安装向导（加载器 → 加载器版本 → 确认）→ 安装进度；设置同样分为分类页与子页，面包屑保留返回路径 |
| 深浅主题 | 深色/浅色两套设计令牌，主题在首帧绘制前应用并写入本地缓存，与启动器设置里的主题同步 |
| 正版登录 | Microsoft OAuth 设备代码流：在浏览器输入一次性代码完成登录，启动器不接触密码；随后走 Xbox Live → XSTS → Minecraft 服务 → 游戏档案与拥有校验。启动前按需刷新令牌，界面与 IPC 都拿不到令牌本身 |
| 账户头像 | 使用官方皮肤纹理（`textures.minecraft.net`）在本机裁出头部，不经过任何第三方头像服务；下载失败时回退为文字头像 |
| 离线账户 | 角色名 3–16 位字母/数字/下划线；UUID 由 `OfflinePlayer:<name>` 推导，与 Java 的 `UUID.nameUUIDFromBytes` 完全一致（有对照测试） |
| 实例隔离 | 每个实例独立存档、模组、配置与日志；共享依赖库、资源与 Java，避免重复占用磁盘 |
| 多游戏目录 | 每个实例一个 `.minecraft`，也可以绑定任意一个已有目录：官启、HMCL、PCL 的安装都能直接用，实例之间互不干扰 |
| 导入已有 .minecraft | 扫描目录里的 `versions/`，识别游戏版本与加载器（原版、Fabric、Forge、NeoForge），注册为实例即可启动。不复制文件：版本文件、依赖库与资源都从原目录读取，只补齐真正缺失的部分 |
| 四种加载器 | 原版、Fabric、Forge、NeoForge。Forge/NeoForge 通过解释官方安装清单并运行官方 processors 完成客户端安装 |
| Java 管理 | 扫描系统与私有运行时，按版本元数据自动选择；缺失时从 Adoptium 下载 JRE/JDK 到启动器私有目录（SHA-256 校验） |
| 可恢复下载 | 暂停/继续安装，重启后断点续传；小文件并发、大文件自动分段，1–16 路总连接上限；有限重试、SHA-1/SHA-256 与大小校验 |
| 下载进度可视化 | 安装与 Java 下载时列出正在传输的具体文件与各自的字节进度条；主进度与浮动任务卡按字节计算，150ms 节流的进度事件不会拖慢下载 |
| BMCLAPI 镜像 | 设置内可选官方源、BMCLAPI 或兼容的自定义镜像；覆盖版本元数据、客户端 JAR、资源与加载器依赖，下载失败自动回源 |
| 离线启动 | 安装完成后启动路径不访问网络；严格离线模式下缺失文件会被直接列出 |
| 模组管理 | 添加、启用/停用（`.jar` ↔ `.jar.disabled`）、删除；拒绝越界文件名 |
| 可控性 | 启动前展示完整命令行、内存与参数、Java 路径、classpath 条目数与缺失文件 |
| 日志 | 实时输出 + 磁盘日志（4 MiB 自动轮转），可刷新与清空 |

## 快速开始

```bash
pnpm install                 # 前端依赖
pnpm tauri dev               # 开发模式（需要系统 WebView 依赖）
pnpm tauri build             # 打包（AppImage/DEB/NSIS/DMG）
```

仅验证核心逻辑（不需要图形环境）：

```bash
cargo test -p launcher-core
pnpm test                    # 前端纯函数单测（版本分类、筛选、主题）

# 真实安装与启动演练（与界面走同一套代码）
cargo run -p launcher-core --example smoke -- plan 1.20.1
cargo run -p launcher-core --example smoke -- install /tmp/cube alice 1.20.1 fabric
cargo run -p launcher-core --example smoke -- verify  /tmp/cube alice --offline
cargo run -p launcher-core --example smoke -- launch  /tmp/cube alice

# 版本目录分类与来源（离线读本地缓存）
cargo run -p launcher-core --example smoke -- catalog ~/.local/share/CubeLauncher

# 正版登录（交互式：在浏览器输入打印出来的代码）
cargo run -p launcher-core --example smoke -- login    /tmp/cube
cargo run -p launcher-core --example smoke -- accounts /tmp/cube
cargo run -p launcher-core --example smoke -- refresh  /tmp/cube Steve

# 扫描一个已有的 .minecraft，再把其中一个版本注册为实例
cargo run -p launcher-core --example smoke -- scan   ~/.minecraft
cargo run -p launcher-core --example smoke -- import /tmp/cube imported ~/.minecraft 1.20.1-forge-47.4.26
cargo run -p launcher-core --example smoke -- verify /tmp/cube imported --offline

# 实测 BMCLAPI 客户端下载与校验（仅下载指定版本的 JAR）
cargo run -p launcher-core --example download_check -- /tmp/cube-download-check 1.20.1
```

系统依赖见 [Tauri 前置条件](https://v2.tauri.app/start/prerequisites/)：Linux 需要
WebKitGTK 4.1 与 GTK 3，Windows 需要 WebView2，macOS 使用系统自带的 WKWebView。

## 目录结构

```
crates/launcher-core/     与界面无关的启动器核心
  src/types.rs            全部数据结构（配置、实例、版本元数据、版本目录、安装清单）
  src/rules.rs            规则求值、Maven 坐标、Java 版本下限、路径越界防护
  src/download.rs         并发下载、续传、校验、镜像改写、进度事件（在传文件与字节进度）
  src/meta.rs             版本清单与目录分类、元数据、继承合并、参数占位符展开
  src/install.rs          安装计划、资源与依赖、原生库解压、Fabric
  src/forge.rs            Forge / NeoForge 安装清单与 processor 执行
  src/java.rs             Java 探测与 Adoptium 运行时安装
  src/launch.rs           启动计划、命令行生成、进程与日志
  src/auth.rs             正版登录：设备代码流、Xbox Live/XSTS、令牌刷新、皮肤纹理
  src/instance.rs         账户、实例、模组
  src/gamedir.rs          游戏目录：布局解析、扫描与导入已有 .minecraft
  examples/smoke.rs       命令行演练工具
src-tauri/                Tauri 适配层（命令、事件、能力声明）
src/App.svelte            界面外壳：侧边栏 + 分级路由
src/components/           页面组件：总览、实例、版本目录、安装向导、导入目录、安装进度、设置
src/lib/                  共享状态、动作与纯函数（版本分类筛选、主题、设置分组）
src/types/api.ts          与 Rust 端一一对应的类型与命令封装
```

数据目录默认是系统应用数据目录下的 `CubeLauncher`，可在设置中修改：

```
CubeLauncher/
├── settings.json         启动器设置
├── accounts.json         离线角色与正版登录令牌（Unix 下权限 0600）
├── instances/<id>/       实例：instance.json、install.lock、.minecraft/、natives/、logs/
├── libraries/            依赖库与版本 JAR（实例共享）
├── assets/               资源对象、资源索引、日志配置
├── metadata/             版本元数据与加载器版本 JSON
├── runtimes/             启动器下载的 Java 运行时
└── logs/                 每个实例的运行日志
```

`instances/<id>/.minecraft/` 是启动器为实例创建的目录。导入的实例不在这里放游戏文件：
`instance.json` 里的 `game_dir` 指向用户自己的 `.minecraft`，启动器只在该实例目录下写原生库与日志。
导入的目录自带 `libraries/`、`assets/` 时，缺失文件的补齐也写回那里，保证 Forge 一类依赖
`${library_directory}` 的加载器只有一个库根目录。

## 兼容范围

启动器本身支持 Windows 10/11 x64、macOS 12+（Intel 与 Apple Silicon）、Ubuntu 22.04+ x64。
具体游戏还受其自身 Java、图形与原生库要求约束。

| 加载器 | 说明 |
| --- | --- |
| 原版 | 解析 1.12.2 起的正式版；Java 需求以版本元数据 `javaVersion` 为准，缺失时按受测回退表推断 |
| Fabric | 版本来自官方 Meta API，默认选择 stable loader；Fabric API 是模组，不由启动器代装 |
| Forge | 支持下限 1.12.2；按安装包内容区分旧式 `install`/`versionInfo` 与现代 `install_profile` 两种格式 |
| NeoForge | `net.neoforged:neoforge` 分支，1.20.2 起；1.20.1 请使用 Forge |

Apple Silicon 上，近期版本按官方原生库使用 ARM Java；旧版本走 Rosetta + x64 Java，
不替换上游 LWJGL 冒充原生支持。未实测的组合不会被标记为“已支持”。

导入的目录按其中的版本文件启动，因此不受上表限制：Fabric、Forge、NeoForge 会被识别并显示版本号，
Quilt、OptiFine、LiteLoader 等没有安装器的加载器会按“原版”标注，但依然能正常启动，
因为启动器只是合并该目录里的 `inheritsFrom` 链并运行它自己的主类。

## 已知限制

- 正版登录使用 OAuth 设备代码流：需要自己打开 `https://microsoft.com/link` 并输入启动器显示的代码，
  启动器不会接触密码；也可以直接点“使用 Microsoft 账户登录”让启动器代开浏览器。
  登录要求该 Microsoft 账户已购买 Minecraft Java 版，没有游戏档案时会明确报错（XSTS 的常见错误码会翻译成人话）。
- 默认使用启动器内置的公共 Microsoft 应用 ID；如果你自己注册了应用（需允许公共客户端流），
  可在“设置 → 账户与登录”里替换。该 ID 属于公共客户端，不涉及密钥。
- 登录令牌（Microsoft 刷新令牌与 Minecraft 访问令牌）以明文保存在数据目录的 `accounts.json`，
  Unix 下写入权限为 0600（有单测锁定）；删除角色即删除令牌。界面与 IPC 只拿到状态，不拿到令牌。
  长期凭证只有刷新令牌，且只会发给 Microsoft 与 Minecraft 官方接口，不会上传到第三方。
- 严格离线模式下不会发起登录或刷新请求：只使用已经缓存的、仍然有效的令牌，过期则提示关闭严格离线模式。
- 不提供 Realms 专用功能、外置登录（authlib-injector / 第三方验证服务器）与 Xbox 好友等社交功能；
  正版账户进入正版验证服务器、显示皮肤与名称都由游戏自身完成。游戏与模组自身的联网行为不受启动器控制。
- 镜像为可选项，默认使用官方源。可在“启动器设置 → 下载与镜像”选择 BMCLAPI，保存后生效；
  自定义地址须提供兼容 BMCLAPI 的路由。版本清单、客户端 JAR、依赖库、加载器 Maven、Fabric 元数据与资源对象均可走镜像，失败时自动回到原地址。
- 下载并发数为全局连接上限（默认 4，建议 4–8，最大 16），大文件自动最多 4 段并行；服务器不支持 Range 时自动使用普通下载。
  暂停或网络中断会保留临时数据，点击“继续安装”或重启后在原实例继续即可恢复；完整性校验失败会重新下载损坏内容。
- 部分镜像的加载器版本列表可能滞后，此时可在安装向导里手动填写版本号。
- 版本目录缓存 6 小时内直接使用；过期后下一次打开会尝试刷新，刷新失败仍继续使用旧缓存并给出提示，
  严格离线模式下手动刷新会直接报错，而不是静默失败。
- 愚人节版本按已知 ID 列表单独分组（与 HMCL 的做法一致）；上游新出现的玩笑版本在收录进该列表前，
  会先留在快照分类里。
- Java 运行时另有独立镜像选项：上游 Adoptium 链接指向 GitHub Release，在部分网络下很慢，
  填写镜像基地址（如 `https://mirrors.tuna.tsinghua.edu.cn/Adoptium`）后可显著加速，
  SHA-256 仍按官方接口校验，失败会自动回退上游。
- 旧版游戏在使用系统 Java 8 之外的运行时可能失败；启动器会给出所用 Java 与缺失项提示。
- 导入是“就地使用”而不是搬迁：启动器不会把外部目录复制进来，也不会在删除实例时删除它。
  同一个目录的同一个版本只允许一个实例使用，否则两个实例会争抢同一份存档与模组。
  补齐缺失文件时，目录自带 `libraries/`、`assets/` 就写回那里，否则写进启动器数据目录；
  原目录里已有的文件不会被改写或删除。
- 代码签名与 macOS 公证需要相应证书，仓库只提供构建能力。

## 文档

- [验证记录](docs/VERIFICATION.md)：逐项命令、输出与结论，包含实测资源占用。
- [构建说明](docs/BUILDING.md)：三平台构建、打包与常见问题。

## 许可

MIT，见 [LICENSE](LICENSE)。
