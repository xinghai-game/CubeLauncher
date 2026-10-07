# 验证记录

本文记录 CubeLauncher 的实测结果、命令与输出。所有数字都来自本机运行，
不是估算值；未实测的内容一律标为“未验证”。

- 日期：2026-10-06（原验证）、2026-10-07（26.x 元数据修复复测、正版登录）
- 机器：Debian（Linux x86_64），16 核，Xvfb 虚拟显示
- 工具链：Rust 1.94.0、Node.js 24.14.1、pnpm 12.8.1、OpenJDK 25（Zulu）
- 数据目录：`/tmp/cube-e2e`

## 1. 单元测试

```bash
cargo test -p launcher-core
# test result: ok. 43 passed; 0 failed
```

覆盖内容：离线 UUID 与 Java 结果逐值对照、规则“最后匹配生效”、继承合并（标量子级覆盖、
参数父→子追加、同身份库子级覆盖、classifier 不误判冲突）、旧式参数引号分词、
未知占位符报错、Maven 坐标与 classifier、Java 版本下限回退表、资源 URL 前缀、
镜像改写、安装清单 data 值分类与 `@扩展名`、处理器变量替换与未知变量报错、
路径越界防护、断点续传文件名稳定、原子写入、26.x 元数据的无 `rules` 参数条目
（`default-user-jvm` 的 `{"value": […]}` 无条件生效）、未知参数形状只跳过不报错。

## 2. 真实安装与启动

全部通过 `cargo run -p launcher-core --example smoke`（与界面同一套核心代码）。
启动测试在 `xvfb-run` 虚拟显示中进行，`timeout` 到期杀掉进程，因此退出码 124
表示“进程一直健康运行到被手动结束”。

| 场景 | 结果 |
| --- | --- |
| 原版 1.20.1 安装 | 3630 个文件（3598 个资源对象）全部下载并校验，`verify` 缺失 0 项 |
| 原版 1.20.1 启动 | 进入主菜单：`Setting user: Alice`、`Backend library: LWJGL version 3.3.1 build 7`、`Sound engine started` |
| 原版 1.20.1 严格离线启动 | 离线模式（禁止任何网络请求）下校验 0 缺失并成功启动 |
| 原版 1.21.1 安装 | 完成，元数据要求 Java 21，启动器按元数据选择 Java |
| 原版 26.3 安装 | 5161 个文件（5147 个资源对象）全部下载并校验，`verify` 缺失 0 项；元数据要求 Java 25，启动器按元数据选择 Java 25 |
| 原版 26.3 启动 | 进入主菜单：`Setting user: Alice`、`Backend library: LWJGL version 3.4.3+4`、`Sound engine started`、纹理图集创建完成；Xvfb 下 OpenGL 不可用时游戏自动切到 Vulkan（`Using graphics backend Vulkan`、`NVIDIA GeForce RTX 2080 SUPER`） |
| Fabric 0.19.5 @ 1.20.1 | 安装 8 个依赖并校验；启动日志 `Loading Minecraft 1.20.1 with Fabric Loader 0.19.5`，Knot/Mixin 正常，0 error |
| Forge 47.4.26 @ 1.20.1 | 依次执行 6 个客户端 processor 并校验产物；启动日志出现 `--launchTarget forgeclient`、classpath 含 `1.20.1-forge-47.4.26.jar`，0 error |
| NeoForge 21.1.255 @ 1.21.1 | 依次执行 6 个客户端 processor；启动日志 `NeoForge 21.1.255 (neoforge)`、`ModLauncher 11.0.5 ... java version 21.0.11`，0 error |
| Forge 14.23.5.2864 @ 1.12.2（旧式） | 从安装器内嵌 `maven/` 提取通用 JAR 到正确的 Maven 路径，写入带 `inheritsFrom` 的版本 JSON；解压 7 个 LWJGL2/jinput 原生库；启动日志出现 `FMLTweaker`、`FMLDeobfTweaker`、`TerminalTweaker` 与 `[forge] at CLIENT`，0 error |
| 托管 Java 8 运行时 | 从 Adoptium 查询、下载、SHA-256 校验、解压到 `runtimes/8-x64-jre/jdk8u504-b01-jre`，识别为 Java 8 / x86_64 |

四个加载器（含 1.12.2 旧式路径）都实际进入了游戏阶段：原版与加载器日志出现纹理图集创建、
声音引擎启动或 ModLauncher 初始化。唯一出现的报错是原版离线启动时的
`InvalidCredentialsException: Status: 401 / Failed to verify authentication`，
这是离线身份的正常表现，不影响进入世界。

### 旧版（1.13 之前）的 classpath

1.12.2 及更早的版本元数据没有 `arguments.jvm` 段，因此**启动器必须自己补 `-cp`**。
这是实测发现的问题：缺少该参数时 Java 报 `找不到或无法加载主类
net.minecraft.launchwrapper.Launch`。修复后 1.12.2 Forge 正常启动。

### 现代 Forge 客户端 JAR 的处理

Forge/NeoForge 的 processor 产出 `{PATCHED}`（例如
`net.minecraftforge:forge:1.20.1-47.4.26:client`），启动器把它放到
`metadata/versions/<版本ID>/<版本ID>.jar`，与官方启动器“版本目录内的 JAR 覆盖继承来的
JAR”的语义一致。因为版本 JSON 的 JVM 参数里有 `-DignoreList=...,${version_name}.jar`，
文件名必须与版本 ID 相同，实测启动日志确认该文件确实在 classpath 上。

## 3. 模组管理

```bash
cargo run -p launcher-core --example smoke -- mods /tmp/cube-e2e alice
```

添加 → 显示为启用；停用 → 变为 `*.jar.disabled`；重新启用 → 恢复 `.jar`；删除 → 目录清空；
传入 `../escape.jar` 被拒绝（`文件名不合法`）。

## 4. 资源占用（release 构建）

```bash
cargo build --release -p cube-launcher   # 二进制 14.8 MiB
```

在 Xvfb 中启动打包后的应用，等待 25 秒稳定后采样 30 秒：

| 进程 | PSS | RSS |
| --- | --- | --- |
| cube-launcher | 70.6 MiB | 189.2 MiB |
| WebKitWebProcess | 84.9 MiB | 196.6 MiB |
| WebKitNetworkProcess | 31.2 MiB | 72.0 MiB |
| **合计** | **184.9 MiB** | 457.7 MiB（共享页重复计算） |

- 空闲 30 秒 CPU 时间：0 tick（< 0.01 核，占 16 核机器的 0.1% 以下）。
- 前端产物：JS 138.9 kB（gzip 39.3 kB）、CSS 22.5 kB（gzip 5.9 kB）。
- 结论：内存目标（≤ 200 MiB）与 CPU 目标（< 1% 单核）达成。
- 注意：测量在虚拟显示中进行，WebKit 未启用 GPU 合成；真实桌面环境的显存与
  合成开销未计入，可能略高。RSS 合计因共享库重复计数而偏大，PSS 才是真实占用。

## 5. 界面验证

打包后的应用在虚拟显示中能启动并创建窗口（`xwininfo` 可见 `CubeLauncher 1240x780`），
但 WebKitGTK 在无 GPU 的 Xvfb 里不绘制 DOM，因此改用 Chromium 渲染同一份构建产物，
并以桩函数提供与 Rust 端同名的命令与真实数据：

| 截图 | 内容 |
| --- | --- |
| `docs/screenshots/home.png` | 总览：最近实例、实例卡片、离线校验/Java/下载统计 |
| `docs/screenshots/instances.png` | 实例管理：列表、启动/校验/修复/目录/删除、运行设置标签页 |
| `docs/screenshots/downloads.png` | 版本目录：分类标签与计数、来源状态、搜索、已安装标记 |
| `docs/screenshots/wizard.png` | 安装向导第 1 步：四种加载器与动态查询到的可用版本 |
| `docs/screenshots/wizard-versions.png` | 安装向导第 2 步：加载器版本列表（最新项带标记） |
| `docs/screenshots/wizard-confirm.png` | 安装向导第 3 步：实例名、需要的 Java、安装位置 |
| `docs/screenshots/install.png` | 安装进度页：阶段轨道、百分比、后台安装入口 |
| `docs/screenshots/install-download-progress.png` | 安装进度页（下载中）：正在下载的具体文件与各自进度条、全量字节进度 |
| `docs/screenshots/download-task-toast.png` | 右下角浮动任务卡：离开安装页后仍显示当前下载的文件与字节进度 |
| `docs/screenshots/settings.png` | 设置分类页：五个分组与当前取值摘要 |
| `docs/screenshots/settings-java.png` | 设置子页：Java 运行时列表、下载与选择规则 |
| `docs/screenshots/theme-light.png` | 浅色主题下的版本目录（设计令牌全部生效） |


## 6. Java 运行时下载与镜像

上游 Adoptium 的下载链接指向 GitHub Release，在本机极慢（41 MiB 的 JRE 40 分钟只完成
约 28 MiB，最终放弃）。同一文件在大学镜像上测得：

```bash
curl -r 0-2000000 -o /dev/null -w '%{http_code} %{speed_download} B/s\n' \
  https://mirrors.tuna.tsinghua.edu.cn/Adoptium/8/jre/x64/linux/OpenJDK8U-jre_x64_linux_hotspot_8u504b01.tar.gz
# 206 6321675 B/s ≈ 6.3 MB/s
```

因此设置里增加了独立的 “Java 镜像” 选项，按
`{base}/{主版本}/{类型}/{架构}/{系统}/{文件名}` 组装地址，SHA-256 仍取自官方接口，
镜像失败会自动回退上游地址（实测：通过镜像数秒完成下载、校验、解压与识别）。

下载器的超时策略也据此调整：只保留连接超时与 90 秒**停滞**超时，
不再对整体传输设上限，否则大文件（客户端 JAR、Java 运行时）会被误杀。

## 7. 打包

```bash
pnpm tauri build --bundles deb
# → target/release/bundle/deb/CubeLauncher_0.1.0_amd64.deb (5.74 MiB)
```

DEB 元信息（`dpkg -I`）：`Package: cube-launcher`、`Architecture: amd64`、
`Depends: libwebkit2gtk-4.1-0, libgtk-3-0`、`Installed-Size: 15211`，
包含 `usr/bin/cube-launcher` 与 `usr/share/applications/CubeLauncher.desktop`。
未在本机执行安装（会改动系统），只检查了包内容。

AppImage 打包失败：Tauri 需要从 GitHub 下载 `AppRun-x86_64`，而本机到 GitHub 的连接
被中断（`peer closed connection without sending TLS close_notify`）。这是本机网络限制，
不是配置问题；`bundle.targets` 已包含 `appimage`，在可访问 GitHub 的环境（如 CI）可直接产出。
NSIS 与 DMG 同理，由 CI 工作流覆盖。

## 8. 本次未验证的内容

- Windows 与 macOS 的构建与启动（仓库提供 CI 工作流与打包配置，但本机无法实测）。
- Apple Silicon 的原生 ARM 与 Rosetta 两条路径。
- 真实桌面（有 GPU 合成）下的内存与渲染表现。
- 微软正版登录、整合包导入、自动更新等未纳入首版的功能。
- 1.13–1.16 区间 Forge（spec 0 + processor）与 1.7.10 等更早版本的实物验证；
  代码按同一套清单解释器处理，但未逐包实测。

## 9. 验证过程中发现并修复的问题

验证不是走过场，以下都是实跑时才暴露出来的缺陷，已修复并复测：

| 问题 | 现象 | 修复 |
| --- | --- | --- |
| 旧版本缺 classpath | 1.12.2 报“找不到或无法加载主类 launchwrapper.Launch” | 元数据没有 `arguments.jvm` 时由启动器补 `-cp` |
| 原生库校验用错哈希 | 1.12.2 的 `lwjgl-platform` 原生库校验失败 | classifier 使用自己的 sha1/size，而不是主构件的 |
| 处理器参数里的 Maven 坐标 | Forge 报 `Input does not exist: [de.oceanlabs.mcp:...@zip]` | 支持在处理器参数中展开 `[坐标@扩展名]` |
| Adoptium 响应结构 | “没有适用于 linux/x64 的 Java 8” | 兼容 `binary`（单数）与 `binaries`（数组）两种结构 |
| 下载总超时 | 大文件在传输中途被中断 | 改为连接超时 + 90 秒停滞超时 |
| 断点续传跨重启失效 | 中断后重新开始下载 | 分片文件名去掉进程号，保持可续传 |
| 模组重复后缀 | 启用已停用的模组时报 `*.jar.disabled.disabled` | 归一化文件名，并允许已是目标状态 |
| Forge 版本号前缀 | 生成 `forge-1.20.1-1.20.1-47.4.26-installer.jar` | 统一 Maven 版本号格式 |
| 镜像元数据滞后 | 镜像的 Forge 版本列表停在 1.18 | 额外读取镜像自己的构建列表并合并去重 |
| 界面空白 | 打包后窗口只有背景色 | Svelte 5 必须用 `mount()` 而不是 `new App()` |
| 事件失败阻断数据加载 | 订阅失败时界面停在空中状态 | 订阅与数据加载分别捕获异常 |
| 26.x 元数据解析失败 | 安装 26.3 报 `解析版本元数据失败: data did not match any variant of untagged enum ArgumentValue at line 1 column 155` | `arguments.default-user-jvm` 的 `{"value": […]}` 条目本来就没有 `rules`：把 `rules` 改为可选（缺省 = 无条件生效），并给未来新增的参数形状留一个“保留但跳过并告警”的兜底；26.1-snapshot-2 起共 59 个版本命中，含当前正式版 26.3 与快照 26.4-snapshot-3 |

## 10. 复现方式

```bash
# 单元测试
cargo test -p launcher-core

# 安装（示例：Fabric）
cargo run -p launcher-core --example smoke -- install /tmp/cube-e2e alice 1.20.1 fabric

# 校验与启动（--offline 表示严格离线）
cargo run -p launcher-core --example smoke -- verify /tmp/cube-e2e alice --offline
xvfb-run -a cargo run -p launcher-core --example smoke -- launch /tmp/cube-e2e alice

# 加载器版本发现
cargo run -p launcher-core --example smoke -- versions forge 1.20.1

# 版本目录分类、来源与 Java 需求（离线读缓存）
cargo run -p launcher-core --example smoke -- catalog ~/.local/share/CubeLauncher
```

Forge 与 Fabric 的 Maven 主机在本机存在 TLS 证书不匹配（证书只对某个 IP 有效），
因此上述命令可附加 `--mirror https://bmclapi2.bangbang93.com`。镜像只改变下载地址，
校验仍然使用官方元数据里的 SHA-1/SHA-256。

## 11. 版本目录动态获取与页面分级（2026-10-07 追加）

参考 HMCL 的两点改造：版本列表动态获取并分类，界面按层级组织。

### 核心与数据

- `launcher-core` 新增 `VersionCatalog`：清单里每个版本带上派生分类（正式版 / 快照 / 远古 Beta /
  远古 Alpha / 愚人节），并记录列表来源（官方源 / 镜像 / 本地缓存）与缓存时间。
- 愚人节分组沿用 HMCL 的做法，用已知 ID 列表判定；本机缓存的真实清单里 8 个 ID 全部命中：
  `15w14a`、`1.RV-Pre1`、`3D Shareware v1.34`、`20w14infinite`、`22w13oneblockatatime`、
  `23w13a_or_b`、`24w14potato`、`25w14craftmine`。
- 缓存策略：6 小时内直接使用；过期时先尝试刷新，刷新失败仍返回旧缓存；`force` 时一定走网络，
  严格离线模式下直接报错而不是静默失败。
- 真实清单规模（`cargo run -p launcher-core --example smoke -- catalog ~/.local/share/CubeLauncher`，
  严格离线、读本地缓存）：918 个版本 = 103 正式版 / 746 快照 / 26 远古 Beta / 35 远古 Alpha /
  8 愚人节；最新正式版 26.3，最新快照 26.4-snapshot-3。这次运行同时验证了过期缓存（> 6 小时）
  在离线下的回退：来源显示“本地缓存（来自缓存：是）”而不是报错。
- 新增 `required_java_major`：版本元数据已在本地时用其中的 `javaVersion`，否则回退到受测版本表，
  并注明来源；不联网、不阻塞界面。同一次运行打印：1.20.1 → Java 17、1.12.2 → Java 8、
  b1.8.1 → Java 8，来源均为 `fallback`（本机没有这三个版本的元数据缓存）。

### 界面

- `src/App.svelte` 变为外壳 + 分级路由，页面拆到 `src/components/`：
  版本目录（分类标签 + 搜索 + 已安装标记 + 加载更多）→ 安装向导（加载器 → 加载器版本 → 确认）
  → 安装进度页；设置改为分类页 → 子页，返回路径显示在面包屑里。
- 向导里的加载器可用性完全来自动态查询：Fabric 走官方 Meta API，Forge/NeoForge 走各自 Maven
  元数据；查不到时给出原因，并允许手动填写版本号。

### 命令与结果

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| Rust 单测 | `cargo test -p launcher-core` | 65 passed; 0 failed（05:14，含对方新增的 gamedir/import 测试） |
| Rust 格式 | `cargo fmt --all -- --check` | 通过（05:14） |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 05:14 仍有 2 处 `unused_imports`，全部位于对方新增的 `download/engine.rs`（`ETAG`/`IF_RANGE`/`LAST_MODIFIED` 与 serde），见文末并发说明 |
| 前端单测 | `pnpm test` | 52 passed（版本分类 12 + 下载/安装 36 + 主题 4） |
| 类型检查 | `pnpm run check` | 0 errors, 0 warnings |
| 前端构建 | `pnpm run build` | JS 208.26 kB（gzip 56.62 kB）、CSS 44.14 kB（gzip 9.34 kB） |
| 版本目录（真实缓存） | `cargo run -p launcher-core --example smoke -- catalog ~/.local/share/CubeLauncher` | 918 个版本，分类计数见上；离线回退到缓存成功 |

界面截图见第 5 节表格，10 张全部由 Chromium 渲染构建产物得到（05:12 用合并后的构建重截，
实例页已包含另一个会话的“导入 .minecraft / 游戏目录”入口）；桩函数提供 `version_catalog`、
`version_java`、`loader_versions` 与安装进度事件，其中 NeoForge 在 1.20.1 上按真实行为返回
“没有找到支持版本”，截图中能看到这条动态结果。

### 截图暴露并修复的问题

| 问题 | 现象 | 修复 |
| --- | --- | --- |
| 面包屑竖排 | 全局 `nav { flex-direction: column }` 同样命中面包屑的 `<nav>` | 面包屑显式声明 `flex-direction: row` |
| 安装页残留分支 | `InstallProgressPage` 保留着旧的“停止游戏”按钮，引用已删除的符号 | 类型检查报错后删除该分支 |

### 并发说明

改造期间另一个会话同时在改主题（`src/lib/theme.ts`、`styles.css` 设计令牌、`index.html`
首帧脚本、`tests/theme.test.ts`）与启动器核心（元数据参数解析的健壮性、`Instance` 增加
`game_dir`/`version_id`、拆出 `gamedir.rs`/`mirror.rs`/`download/engine.rs` 等模块），
之后又在本节的组件结构上继续扩展（新增 `ImportPage.svelte` 与 `src/lib/install.ts`、
`downloads.ts`，实例页增加“导入 .minecraft / 游戏目录”）。
两边改动都已保留：主题按钮接到 `chooseTheme`，本次的版本目录代码在对方重写后的
`meta.rs`/`types.rs`/`src-tauri/src/lib.rs` 里依然完整，页面仍按同一套分级路由渲染。

Rust 侧在 **2026-10-07 05:14** 重新验证：`cargo test` 65 项通过、`cargo fmt --all -- --check`
干净；`catalog` 那一行是 05:10 实测。唯一没有通过的是 `cargo clippy --all-targets -- -D warnings`：
`download/engine.rs` 报 `ETAG`、`IF_RANGE`、`LAST_MODIFIED` 与 serde 的 `unused_imports`。
该文件属于对方正在重写的下载器，这些导入未被使用说明它可能刚移走断点续传相关代码（也可能只是
还没接回来），因此没有代为删除——等对方收尾后重跑一次 clippy 即可确认。

对方在重写 `examples/smoke.rs` 时保留了我新增的 `catalog` 子命令，它同时新写的 `scan` 命令里
少了一个 `{}`（8 个参数对 7 个占位符），会让 `cargo clippy --all-targets` 直接失败；已补上该占位符，
`catalog` 因此得以实际运行。`download/engine.rs` 里的 `nonminimal_bool` 属于同类机械问题，一并修掉。

## 12. 多 `.minecraft` 目录与导入已有安装（2026-10-07 追加）

每个实例仍然拥有一个游戏目录，但现在可以指向用户自己的 `.minecraft`：导入不复制文件，
启动时从原目录读取版本文件、依赖库与资源。核心改动集中在新的
[`crates/launcher-core/src/gamedir.rs`](../crates/launcher-core/src/gamedir.rs)。

### 设计

- `Instance` 新增 `version_id`（`versions/` 里的版本文件夹名）与 `game_dir`（外部 `.minecraft`）。
  两者都带 `#[serde(default)]`，旧实例文件照常解析，行为不变。
- `InstanceLayout` 把“读哪里、写哪里”一次性解析出来：`libraries`/`assets` 的写目标是导入目录
  自己的文件夹（存在时），读顺序是导入目录 → 启动器缓存；`versions` 与版本 JAR 的读顺序里
  加上导入目录，写目标仍然是启动器缓存（不去动别人 `versions/` 里的东西）。
- 因此 `${library_directory}` 只会指向一个真实存放这些 JAR 的目录。这一点是必须的：现代 Forge 的
  `-p` 模块路径由 `${library_directory}/cpw/mods/...` 拼出，库分散在两处就会启动失败。
- `resolved_for` 优先读实例自己的版本文件，`inheritsFrom` 的父版本也从同一目录解析，
  所以导入的安装**不需要**启动器的元数据缓存，也不需要联网。
- 加载器识别以库坐标为准（`net.neoforged:neoforge`、`net.neoforged.fancymodloader:loader`、
  `net.minecraftforge:fmlloader`、`net.minecraftforge:forge`、`net.fabricmc:fabric-loader`），
  版本号优先取版本文件夹名——NeoForge 的 `neoforge-21.1.255` 只有文件夹名带 21.1.255，
  FancyModLoader 的坐标只知道它自己的 4.0.45。
- 没有安装器的加载器（Quilt、OptiFine、LiteLoader）按“原版”记录并在导入页给出说明，
  但仍然可以启动：启动器只是合并版本文件里的继承链并运行它自己的主类。

### 单元测试

`cargo test -p launcher-core`：**68 passed; 0 failed**（05:33，含对方同时新增的用例），其中 8 项为本节新增：

- 目录识别（选中父目录时下探 `.minecraft`、选中 `.minecraft` 时原样使用、非游戏目录报错）
- 加载器识别（现代 Forge 的 `fmlloader`、1.12.2 的 `1.12.2-forge1.12.2-14.23.5.2864`、
  NeoForge 的 `neoforge-21.1.255`、Fabric、无库时退回版本 ID、Quilt 提示）
- 游戏版本推导（`inheritsFrom` 优先，否则从版本 ID 去掉加载器部分）
- 扫描统计（嵌套目录、版本数、模组/存档计数、已导入标记）
- 就地导入（实例目录里不产生 `.minecraft`、安装记录指向外部版本文件、重复导入被拒、删除实例不影响外部目录）
- 离线解析（父版本文件来自导入目录，无网络也能合并出完整链）
- 计划裁剪（导入目录里已有的文件不再下载；同名但哈希不同的文件不会被误判）
- 绑定与解绑（绑定后 `installed=true` 并写锁；解绑回到启动器目录、`installed=false`、锁被移除）

### 端到端实测

夹具是一个“别人家的 `.minecraft`”：`versions/1.20.1`（原版）与 `versions/1.20.1-forge-47.4.26`
（真实 Forge 版本文件 + 补丁 JAR）、`libraries`、`assets` 与 `mods/`、`saves/`。
启动器数据目录是全新的，没有任何元数据缓存。

```bash
cargo run -p launcher-core --example smoke -- scan   /tmp/cube-import/official
cargo run -p launcher-core --example smoke -- import /tmp/cube-import/data2 imported /tmp/cube-import/official 1.20.1-forge-47.4.26
cargo run -p launcher-core --example smoke -- verify /tmp/cube-import/data2 imported --offline
xvfb-run -a cargo run -p launcher-core --example smoke -- launch /tmp/cube-import/data2 imported --offline
```

| 项目 | 实测输出 |
| --- | --- |
| `scan` | `游戏目录 /tmp/cube-import/official/.minecraft（取自内层 .minecraft，模组 1 个，存档 1 个，自带 libraries=true assets=true）`；两个版本：`1.20.1 → 1.20.1 / 原版 · 有 JAR`、`1.20.1-forge-47.4.26 → 1.20.1 / Forge 47.4.26 · 有 JAR` |
| `import` | `已导入实例 imported（1.20.1 / Forge / 版本 1.20.1-forge-47.4.26）`，`校验结果：0 个缺失项`；`instance.json` 里 `game_dir=/tmp/cube-import/official/.minecraft`、`version_id=1.20.1-forge-47.4.26`，实例目录下只有 `natives/`、`logs/` 与两个 JSON |
| 重复导入 | `Error: 这个目录的 1.20.1 已经由实例 “vanilla-imported” 使用，请选择别的版本或先删除该实例` |
| 严格离线校验 | 数据目录没有元数据缓存，`verify --offline` 仍然 `缺失 0 项`（父版本 1.20.1 从导入目录读到） |
| 启动（`--offline`） | `ModLauncher 10.0.9 … starting: java version 17.0.19`、`Launching target 'forgeclient'`、`Setting user: Alice`、`Backend library: LWJGL version 3.3.1 build 7`、`MinecraftForge v47.4.26 Initialized`、`Sound engine started`，即进入主菜单 |
| 命令行抽查 | `-DlibraryDirectory=/tmp/cube-import/official/.minecraft/libraries`、`-p` 指向同一目录、`--gameDir/--assetsDir` 也是导入目录；classpath 82 项全部来自导入目录 |
| 模组管理 | `smoke mods` 在导入实例上报告的模组目录是 `/tmp/cube-import/official/.minecraft/mods`，添加/停用/启用/删除与越界文件名拒绝均正常 |

修复缺失文件的写回行为（把导入目录里的 `net/minecraftforge/unsafe/0.2.0/unsafe-0.2.0.jar`
删掉后重新导入）：

```
校验结果：1 个缺失项 ["unsafe-0.2.0"]
[install] download 0/0 导入目录已提供 3657 个文件，需要补齐 1 个
[install] download 0/1 下载 1 个文件（含 3598 个资源）
[install]     done 1/1 Forge 47.4.26 安装完成
校验结果：0 个缺失项
```

下载的文件落在导入目录的 `libraries/net/minecraftforge/unsafe/0.2.0/` 下，
启动器缓存里 `libraries` 文件数为 0——即“目录自带 libraries 就写回那里”。

### 界面

| 截图 | 内容 |
| --- | --- |
| `docs/screenshots/instances-imported.png` | 实例列表：导入实例带“外部目录 …/alice/.minecraft”，操作区新增“游戏目录 / 更换目录” |
| `docs/screenshots/import.png` | 导入页第 1、2 步：目录输入与扫描摘要（读取方式、可导入版本、自带 libraries/assets、模组与存档计数）、版本列表与 `含 JAR` 标记 |
| `docs/screenshots/import-confirm.png` | 导入页第 3 步：版本、游戏版本、加载器、角色、游戏目录与简写，底部“导入为实例” |

截图同样由 Chromium 渲染构建产物得到，桩函数按真实输出返回 `scan_game_dir` 的结果
（含 Quilt 的提示行）；`pnpm run check` 为 0 errors / 0 warnings。

### 验证中发现并修复的问题

| 问题 | 现象 | 修复 |
| --- | --- | --- |
| 客户端 JAR 重复下载 | 导入一个完整的 `.minecraft` 后点“修复/更新”，仍会下载 23 MiB 的客户端 JAR | 客户端 JAR 的写目标是启动器缓存，而裁剪逻辑按“导入目录的 libraries”做前缀匹配，于是漏判；`InstanceLayout` 增加 `jars` 写目标并把 `libraries/versions` 映射到 `versions` 读顺序，现已在单元测试里锁定 |
| Forge 版本号识别不到 | 现代 Forge 版本文件里没有 `net.minecraftforge:forge`，只有 `fmlloader`，实例的 `loader_version` 为空 | 识别表补上 `fmlloader`、`fancymodloader:loader` 与 `net.neoforged:forge`（1.20.1 的 NeoForge），版本号优先从版本文件夹名解析 |
| 加载器识别被“原版”吞掉 | 只有版本 ID 能说明加载器时（库被剥离）识别结果没有版本号 | 版本 ID 解析补充 `-forge-`、`-neoforge-`、`fabric-loader-<v>-<game>` 三种形态 |

并发说明（接第 11 节）：本节改动与另一个会话的下载器重写同时进行。对方在 `InstancesPage.svelte`
里新增“查看安装进度”按钮时引用了未导入的 `go`，`pnpm run check` 因此报错，我补上了该导入；
对方则修好了我 `smoke.rs` 里 `scan` 命令少一个 `{}` 的格式串。两边改动都已保留。

Rust 侧在本节收尾时重新验证过一遍（对方的下载器重写完成后）：

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| 单测 | `cargo test -p launcher-core` | 68 passed; 0 failed（05:33） |
| 全工作区类型检查 | `cargo check --workspace` | Finished，无 error |
| 格式 | `rustfmt --check`（本节涉及的文件） | 无差异 |
| 导入端到端 | `smoke scan/import/verify` | 复跑通过，离线缺失 0 项 |
| 补齐缺失文件 | `smoke install`（删掉一个 Forge 依赖后） | 只下载 1 个文件，写回导入目录，启动器缓存 0 文件 |
| 类型检查 / 单测 / 构建 | `pnpm run check`、`pnpm test`、`pnpm run build` | 0 errors；56 passed（新增 `tests/gamedir.test.ts` 4 项：导入判定、目录推导、路径简写、体积格式）；构建成功 |

过程中有一次 `cargo check --workspace` 停在对方 `download/engine.rs` 的 `E0733`（递归 async fn 需要
`Box::pin`）上，那是对方重写到一半的中间状态，对方随后自行修好，与本节的代码无关。

## 13. 正版（Microsoft）登录（2026-10-07 追加）

### 认证链路

`crates/launcher-core/src/auth.rs` 一个文件实现整条链路，每一步都有本地桩服务器的端到端测试
（`cargo test -p launcher-core auth::`，17 项；桩服务器用 `tokio::net::TcpListener` 起在随机端口，
把 `AuthEndpoints` 指过去，不碰真实网络）：

| 环节 | 请求 | 测试覆盖 |
| --- | --- | --- |
| 设备代码 | `POST /consumers/oauth2/v2.0/devicecode`，scope `XboxLive.signin offline_access` | 解析 `user_code`/`interval`；`interval=0` 夹到 1 秒，避免对 Microsoft 忙等 |
| 轮询 | `POST /token`，`grant_type=urn:ietf:params:oauth:grant-type:device_code` | `authorization_pending` → 等待、`slow_down` → 放慢、`expired_token`/`bad_verification_code` → 过期、`authorization_declined` → 取消；未知错误保留 Microsoft 的 `error`/`error_description` 首行。设备代码一旦兑换成功就不能再轮询，因此后续任何失败都返回终态 `failed`（界面立即停止轮询并显示原因），只有真正的传输错误才是可重试的 `Err` |
| Xbox Live | `POST user.auth.xboxlive.com/user/authenticate` | `RpsTicket: d=<令牌>`；被拒时用 `t=` 重试一次（旧账户），断言两次请求体 |
| XSTS | `POST xsts.auth.xboxlive.com/xsts/authorize` | `XErr` 2148916233/235/238/227 翻译成中文提示，未知码回退原文 |
| Minecraft 登录 | `POST api.minecraftservices.com/authentication/login_with_xbox` | `identityToken: XBL3.0 x=<uhs>;<xsts>` |
| 游戏档案 | `GET /minecraft/profile`（Bearer） | 404 → “没有 Minecraft Java 版”；皮肤取 `state=ACTIVE` 的那一张 |
| 拥有情况 | `GET /entitlements/mcstore`（Bearer） | `product_minecraft` / `game_minecraft*`；该接口失败不影响登录（档案已证明拥有） |
| 令牌刷新 | `POST /token`，`grant_type=refresh_token` | 响应省略新刷新令牌时保留旧的；`invalid_grant` → 提示重新登录 |
| 皮肤纹理 | `GET textures.minecraft.net/texture/<hash>` | 校验 PNG 魔数与大小上限，转成 data URL；非 PNG/过大/非 HTTP 一律拒绝 |

一次成功的登录请求序列是 8 个请求（3 次轮询 + 5 跳），测试里逐个断言了方法与 bearer 头。

### 真实端点探测

设备代码与轮询是对着真实 Microsoft 端点跑通的（没有在页面上完成登录，所以一直停在等待）：

```bash
cargo run -p launcher-core --example smoke -- login /tmp/cube-auth-probe
# 请在浏览器中打开 https://www.microsoft.com/link 并输入代码 377HZPNV
# （900 秒内有效，每 5 秒查询一次，Ctrl+C 可以放弃）
# 等待用户在 Microsoft 页面完成登录……
# 等待用户在 Microsoft 页面完成登录……
```

| 端点 | 请求 | 响应 |
| --- | --- | --- |
| `login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` | 代码内 | 200，返回 `user_code`（两次探测分别得到 9FKZVPEF、377HZPNV） |
| `user.auth.xboxlive.com/user/authenticate` | `POST {}` | 400（可达） |
| `xsts.auth.xboxlive.com/xsts/authorize` | `POST {}` | 400（可达） |
| `api.minecraftservices.com/minecraft/profile` | `POST {}` | 401（可达） |
| `api.minecraftservices.com/entitlements/mcstore` | `POST {}` | 401（可达） |

刷新失败路径也对着真实端点验证过（故意写入无效刷新令牌）：

```bash
cargo run -p launcher-core --example smoke -- accounts /tmp/cube-auth-probe
# 共 1 个角色：
#   [正版] Steve 069a79f4-44e9-4726-a5be-fca90e38aaf5 拥有 Java 版：true xuid=2535412345678901 令牌到期：未知 刷新令牌：24 字符
cargo run -p launcher-core --example smoke -- refresh /tmp/cube-auth-probe Steve
# Error: “Steve”的登录已过期，请在账户管理中重新登录
```

### 令牌不出进程

界面拿到的是 `AccountView`（`src-tauri/src/lib.rs`）：名称、UUID、是否拥有 Java 版、令牌到期时间、
令牌是否仍然有效、是否保存了刷新令牌——没有令牌本身。`cargo test -p cube-launcher --lib` 的 3 项
断言序列化结果不含 `access_token` / `refresh_token`，过期令牌会被标成需要刷新，离线角色没有 `microsoft` 块。

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| 核心单测 | `cargo test -p launcher-core` | 97 passed; 0 failed（新增 auth 17、账户存储 4、启动身份 2；其余为原有用例与另一个会话的下载器用例） |
| Tauri 层单测 | `cargo test -p cube-launcher --lib` | 3 passed; 0 failed |
| 全工作区检查 | `cargo check --workspace` | Finished，无 error |
| 前端单测 | `pnpm test` | 73 passed（新增 `tests/account-modal.test.ts` 4 项：设备代码显示与两轮轮询、皮肤头像与刷新按钮、取消登录后不再轮询、离线角色添加；其余含另一个会话在同一时段新增的组件测试，数字为本次收尾时的快照） |
| 类型检查 | `pnpm run check` | 0 errors / 0 warnings |
| 构建 | `pnpm run build` | 成功（3542 modules） |

### 界面

截图沿用第 12 节的方式（Chromium 渲染构建产物 + 与 Rust 同名命令的桩函数），
桩函数这次额外画了一张 64×64 皮肤贴图当数据 URL，因此头像是真实裁剪出来的：

| 截图 | 内容 |
| --- | --- |
| `docs/screenshots/account-login.png` | 账户管理 → 正版登录：设备代码 `K7QP2-9XMTD`、打开链接与复制按钮、等待提示；下方角色列表带“正版/离线”徽标、官方皮肤头部与“登录有效至 …” |
| `docs/screenshots/settings-account.png` | 设置 → 账户与登录：角色与令牌状态、令牌文件位置与 0600 权限、固定的 Microsoft 登录配置 |

### 与离线身份并存的细节

- 启动参数：正版账户 → `--accessToken <Minecraft 令牌> --uuid <无横线 UUID> --xuid <XUID> --userType msa --clientId <启动器常量>`；
  离线角色保持 `--accessToken 0 --userType legacy`（`cargo test -p launcher-core launch::` 两项锁定，
  断言的是版本元数据里 `${auth_*}` 占位符展开后的真实命令行片段）。
- 续期只在启动前发生：剩余有效期不足 5 分钟才联网刷新；严格离线模式下只用已缓存且仍有效的令牌，
  否则明确报“需要联网刷新”，而不是静默用过期令牌启动。刷新后的令牌立即写回 `accounts.json`。
- 账户文件：`atomic_write_private` 先以 0600 写好临时文件再改名（测试直接 `stat` 权限位）；
  旧版只有离线角色的 `accounts.json` 原样可读；同一 UUID 重复登录就地更新，不产生重复条目。
- 头像不经过第三方服务：皮肤纹理从官方 `textures.minecraft.net` 取一份，转 data URL 后在界面里
  用 CSS 裁出头部（脸在 8,8，帽子层在 40,8）；取不到就退回文字头像，不影响任何功能。

### 并发说明（接第 12 节）

本节与另一个会话同时进行。对方在 06:01 前后正在重构 `install.rs`，期间工作区一度无法编译
（调用了尚未写好的 `asset_object_relative`），我先把 `src-tauri`、前端与测试写完，对方补上该函数后
本节的所有 Rust 验证复跑通过。共享文件（`types.rs`、`instance.rs`、`launch.rs`、`src-tauri/src/lib.rs`、
`src/lib/*.ts`、`styles.css`、`README.md`）两侧改动都已保留，合并后 `cargo test`、`pnpm test`、`pnpm check`
三项均通过。

### 本节未验证

| 项目 | 原因 |
| --- | --- |
| 一次真实的完整正版登录（档案、拥有情况、皮肤下载、在线服务器联机） | 需要真实的 Microsoft 账户凭据，本机没有；链路各环节由桩服务器测试覆盖，设备代码与轮询已对真实端点跑通 |
| 正版验证（online-mode）服务器上实际进入游戏 | 同上，需要一个正版账户与一台服务器 |
| Windows / macOS 上的设备代码流程 | 只在 Debian x86_64 上实测；代码路径与平台无关，但未实测 |
| `--clientId` 取值对 Realms / 遥测的影响 | 该值按官方启动器的语义取启动器级常量，只验证了格式与稳定性，没有 Realms 账户可测 |

## 14. 实例操作按钮“点不动”（2026-10-07 追加）

现象：实例页的“校验”“修复/更新”（以及“继续安装/重试安装”等）点下去没有任何反应。

### 成因

| 问题 | 现象 | 修复 |
| --- | --- | --- |
| `installDisabled` 与任何未结束的 task 绑定 | `app.task` 只要有一条 `done=false` 的任务，`修复/更新` 就被禁用且**不会恢复**：`install_java` 只发进度、从不发终态任务，任何一次 Java 下载（成功或失败）都会把它永久卡住 | 后端 `install_java` 的命令层补上 `progress.finish/fail`；前端把忙碌判定改成按实例（`installInFlight`），别人的任务不再冻结本实例的按钮 |
| 禁用态没有样式 | `.button.ghost` 没有 `:disabled` 规则，且 `button { cursor: pointer }` 仍然生效，被禁用的按钮看起来完全正常，点了也没有任何提示 | 全局补 `button:disabled { cursor: not-allowed; opacity: .55 }`，并把 `.button.*:hover` 限定为 `:not(:disabled)` |
| “校验”没有过程反馈 | 校验要扫几千个文件，期间按钮没有任何变化，只有右下角一条 toast，看起来像没反应 | 新增 `app.verifyingId`：校验期间按钮变“校验中…”并转圈，结果无论如何都会提示 |
| 点击后先跳页再判断 | `installSelected` 先 `go('install')` 再发请求，请求被拒时用户会停在“当前没有进行中的安装”的空页上 | 先确认安装真的启动（或本实例正在安装=查看进度）再跳页 |
| `app.busy` 覆盖整个游戏会话 | `launch_instance` 直到游戏退出才 resolve，`app.busy` 因此长期为真，导入/向导/下载 Java 等按钮在游戏运行期间全部静默失效 | 改用 `app.launchingId`：进程状态事件一到就释放，不再用一个全局标志代表整局游戏 |

### 验证

| 项目 | 命令 / 方式 | 结果 |
| --- | --- | --- |
| 单元测试 | `npx vitest run` | 73 passed（新增 `installInFlight` 7 项与实例页组件测试 5 项） |
| 类型检查 | `pnpm run check` | 0 errors, 0 warnings |
| 构建 | `pnpm run build` | 成功（228.49 kB JS / 48.79 kB CSS） |
| 真实界面（Chromium 无头 + `__TAURI_INTERNALS__` 桩） | 注入一条永不结束的 `java` 任务后点击按钮，并读取 `disabled`/`opacity`/`cursor`/`elementFromPoint` 与桩记录的调用 | 11/11 通过：脏任务下“修复/更新”“校验”仍可点；点击“校验”变“校验中…”且 `opacity=0.55 / cursor=not-allowed`，随后调用 `verify_instance` 并提示结果；点击“修复/更新”调用 `install_instance` 并进入安装页；本实例安装中才禁用 |
| Rust | `cargo check -p cube-launcher` | Finished，无 error |

组件测试需要 `svelte` 解析到客户端构建，因此新增了 `vitest.config.ts`（`resolve.conditions: ['browser']`），
`include` 同时覆盖 `tests/**` 与 `src/**/*.test.ts`。


## 15. 下载时显示具体文件与进度（2026-10-07 追加）

之前的进度事件只在**一个文件下载完成后**发送一条消息，界面看到的是“上一条完成的文件名”，
大文件的进行中状态完全不可见；百分比按文件个数计算，单个 100 MiB 的 Java 运行时
在一瞬间从 0% 跳到 100%。本次在进度事件里加入字节级账单与“正在传输的文件”列表：

- `InstallTask` 新增 `downloaded_bytes` / `total_bytes` / `active`（文件名、已下载、总大小）；
- `Progress` 内维护批次账单：`total_bytes` = 所有已声明大小的文件之和（含计划时已存在的文件），
  `done` 由“已结算文件 + 在传文件已接收字节”推导，分段下载的 4 个 worker 共享同一文件条目；
- 传输中每写入一个分块就累加一次，但**节流到 150ms 一条**且用 `try_send`（通道满直接丢弃中间帧，
  慢速 UI 绝不会拖慢下载）；阶段/完成事件不受节流，批次全部结束时立即补发一条落满 100%；
- 界面按字节百分比显示主进度条，逐行展示当前下载的文件（名称过长时显示末段路径、悬停看全路径），
  右下角浮动卡在离开安装页后同样显示“正在下载 <文件>”与字节进度。

### 验证

| 项目 | 命令 / 方式 | 结果 |
| --- | --- | --- |
| Rust 单测 | `cargo test -p launcher-core`（新增 3 项） | 97 passed; 0 failed |
| 进度事件 | 新测试用本地 HTTP 服务器按 40ms/32KiB “滴流”发送 512 KiB 文件 | 事件列表里能捕获到 `active` 出现该文件、`0 < downloaded < total` 的中间帧；字节计数单调不减；终态帧 `downloaded == total`、`active` 为空 |
| 批次字节账单 | 计划中已存在的文件 + 新下载文件混合 `download_all` | 终态 `current=2`、`downloaded_bytes == total_bytes == 64 KiB` |
| 失败清理 | `DISCONNECT` 模式让下载失败 | 最后一条事件 `active` 为空（失败文件离开列表），保留的临时字节仍计入 |
| 类型检查 / 单测 / 构建 | `pnpm run check`、`pnpm test`、`pnpm run build` | 0 errors / 0 warnings；73 passed（其中新增下载展示 5 项）；构建成功 |
| 真实界面（Chromium 无头 + `__TAURI_INTERNALS__` 桩） | 注入一条 5 文件并发的 `download` 任务后读取 DOM | 头部“正在下载 Java 21 等 5 个文件”（取最大文件）、主进度 67%（342/512 MiB）、摘要“342.0 MiB / 512.0 MiB · 2314/3630 个文件”、每行按大小降序（41.0/6.0/4.2/1.4 MiB，进度条 9%/50%/24%/52%）、无大小文件仅显示已下载量、“还有 1 个文件正在并行下载”；总览页浮动卡显示同一份信息；无 console error |
| 截图 | `docs/screenshots/install-download-progress.png`、`download-task-toast.png`（见第 5 节） | OCR 抽查确认字节金额与文件行已渲染 |
