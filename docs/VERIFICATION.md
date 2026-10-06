# 验证记录

本文记录 CubeLauncher 的实测结果、命令与输出。所有数字都来自本机运行，
不是估算值；未实测的内容一律标为“未验证”。

- 日期：2026-10-06
- 机器：Debian（Linux x86_64），16 核，Xvfb 虚拟显示
- 工具链：Rust 1.94.0、Node.js 24.14.1、pnpm 12.8.1、OpenJDK 25（Zulu）
- 数据目录：`/tmp/cube-e2e`

## 1. 单元测试

```bash
cargo test -p launcher-core
# test result: ok. 36 passed; 0 failed
```

覆盖内容：离线 UUID 与 Java 结果逐值对照、规则“最后匹配生效”、继承合并（标量子级覆盖、
参数父→子追加、同身份库子级覆盖、classifier 不误判冲突）、旧式参数引号分词、
未知占位符报错、Maven 坐标与 classifier、Java 版本下限回退表、资源 URL 前缀、
镜像改写、安装清单 data 值分类与 `@扩展名`、处理器变量替换与未知变量报错、
路径越界防护、断点续传文件名稳定、原子写入。

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
- 真实清单规模（本机缓存，2026-10-06 抓取）：917 个版本 = 103 正式版 / 753 快照 /
  26 远古 Beta / 35 远古 Alpha。
- 新增 `required_java_major`：版本元数据已在本地时用其中的 `javaVersion`，否则回退到受测版本表，
  并注明来源；不联网、不阻塞界面。

### 界面

- `src/App.svelte` 变为外壳 + 分级路由，页面拆到 `src/components/`：
  版本目录（分类标签 + 搜索 + 已安装标记 + 加载更多）→ 安装向导（加载器 → 加载器版本 → 确认）
  → 安装进度页；设置改为分类页 → 子页，返回路径显示在面包屑里。
- 向导里的加载器可用性完全来自动态查询：Fabric 走官方 Meta API，Forge/NeoForge 走各自 Maven
  元数据；查不到时给出原因，并允许手动填写版本号。

### 命令与结果

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| Rust 单测 | `cargo test -p launcher-core` | 43 passed; 0 failed |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 无警告 |
| Rust 格式 | `cargo fmt --all -- --check` | 通过 |
| 前端单测 | `pnpm test` | 16 passed（版本分类筛选 12 + 主题 4） |
| 类型检查 | `pnpm run check` | 0 errors, 0 warnings |
| 前端构建 | `pnpm run build` | JS 182.95 kB（gzip 49.51 kB）、CSS 43.07 kB（gzip 9.07 kB） |

界面截图见第 5 节表格，10 张全部由 Chromium 渲染构建产物得到；桩函数提供 `version_catalog`、
`version_java`、`loader_versions` 与安装进度事件，其中 NeoForge 在 1.20.1 上按真实行为返回
“没有找到支持版本”，截图中能看到这条动态结果。

### 截图暴露并修复的问题

| 问题 | 现象 | 修复 |
| --- | --- | --- |
| 面包屑竖排 | 全局 `nav { flex-direction: column }` 同样命中面包屑的 `<nav>` | 面包屑显式声明 `flex-direction: row` |
| 安装页残留分支 | `InstallProgressPage` 保留着旧的“停止游戏”按钮，引用已删除的符号 | 类型检查报错后删除该分支 |

### 并发说明

改造期间另一个会话同时在改主题（`src/lib/theme.ts`、`styles.css` 设计令牌、`index.html`
首帧脚本、`tests/theme.test.ts`）与元数据参数解析的健壮性（`ArgumentValue` 支持缺省 `rules`
与未知形状）。两边改动都已保留：主题按钮接到 `chooseTheme`，Rust 侧统一跑过 `cargo fmt`。
上面的数字是 2026-10-07 04:57 前后的快照；若那个会话继续改动同一批文件，需要重新执行本节命令。
