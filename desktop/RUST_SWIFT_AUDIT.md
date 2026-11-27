# Windows Rust 与原版 Swift 对照

本文覆盖 `desktop/src-tauri` 的 91 个 Rust 文件。各表列出对应的 macOS 实现和 Windows 职责。查某个符号时，还需要确认 Windows 怎样编译它。[内部接口契约](CONTRACTS.md)

- `build-swift.py` 的 `CORE`、`APP`、`TESTS` 清单直接引用仓库中的文件。SwiftPM 通过目录链接访问 `Sources/` 和 `Tests/`，Windows 使用 NTFS junction。例如 `HookIntake.stdout(forAnswer:)`、`SessionSnapshot` 的状态归并和 `CodexAppServerClient.parseMessage` 都由原文件提供。[编译输入](scripts/build-swift.py)
- `swift_sources.py` 仍会将部分函数和语句复制到构建目录的 `Selected*.swift`。其中包括 `HookServer.intake`、`AppState` 的审批和问题回包、`ConfigInstaller` 的安装规则、`RemoteInstaller` 的脚本文本，以及 `JSONLTailer` 的记录解析。下表提到这些符号时，只表示已经找到对应关系。要移除这批副本，还需要为原文件依赖的 macOS 接口或私有状态提供可用入口。[剩余生成规则](scripts/swift_sources.py)
- OpenCode、Pi、OMP 和 OpenClaw 的插件另有 JavaScript/TypeScript 生成步骤。它会读取插件源码并生成 Windows 版本。[插件构建脚本](scripts/plugin_sources.py)

## 从 protocol.rs 到原版入口

`protocol.rs` 定义 Windows 命名管道的请求 ID、超时、应答关联和旧版 `Event` 帧。对于新版 `Hook` 帧，`ipc_handler.rs` 将原始 JSON 传给 `hook.receive`。这里调用的 `HookServer.intake` 仍来自生成文件。发送端的 `HookIntake`、Gemini 的 `stdout(forAnswer:)` 和会话更新所用的 `SessionSnapshot` 已经直接编译原文件。[协议帧](src-tauri/crates/codeisland-ipc/src/protocol.rs) · [接收端](src-tauri/crates/codeisland-runtime/src/ipc_handler.rs) · [Swift 接口](swift/Sources/HookServerAdapter.swift)

可在仓库根目录用下面的命令验证其中一个文件。最后一条比较文件系统标识。即使两个文件的内容相同，副本也会返回 `False`。

```sh
python desktop/scripts/build-swift.py test --scratch .build/direct-audit
python -c "from pathlib import Path; print(Path('.build/direct-audit/package/Targets/CodeIslandCore/Original/HookIntake.swift').samefile('Sources/CodeIslandCore/HookIntake.swift'))"
```

原版 Git 测试在桌面测试目标中调用应用实际使用的路径适配器，覆盖带盘符的 worktree 和 submodule 指针。适配器首先调用原版读取器，并将 HEAD 解析委托给原版。macOS 测试目标照常验证原入口。Codex 的进程生命周期用例固定启动 `/bin/sh`，因此在 Linux 和 macOS 运行，其余 Codex 用例也在 Windows 运行。[Git 测试入口](swift/Tests/GitBranchReaderPlatform.swift) · [路径适配器](swift/Platform/CorePlatform.swift)

原版 Cursor 元数据用例直接调用 `extractMetadata`。它以 `/` 拼接项目路径，在 Windows 上会得到 `/C:/…`。该断言在 macOS 和 Linux 上运行。Windows 通过 `DesktopSessions.apply` 接收含盘符或 UNC 路径的 hook，在调用原版归并函数前传入工作区路径。桌面测试通过这个入口检查两个结果。[原版用例](../Tests/CodeIslandCoreTests/DerivedSessionStateTests.swift) · [桌面入口测试](swift/Tests/DesktopCursorPathTests.swift)

## 仍在复制的 Swift 源码

下表覆盖 `swift_sources.py` 生成的全部 23 个产品源码文件。第一列是 `Selected*.swift` 文件名的中间部分，最后一列给出当前构建不能直接调用相应实现的具体原因。

| 生成文件 | 所需实现 | 直接引用原文件的阻塞 |
| --- | --- | --- |
| `HookServer`、`Webhook` | 事件分流、cwd 过滤、自动审批、Webhook 正文 | `HookServer` 持有 `AppState` 和 Unix socket 监听器。Webhook 正文在私有 `forwardEventToWebhook` 里生成，随后直接发送。自动审批来源解析在 macOS 设置的 getter 内。[HookServer](../Sources/CodeIsland/HookServer.swift) · [Settings](../Sources/CodeIsland/Settings.swift) |
| `HookAnswers`、`PermissionCalls` | 审批、问答、跳过和工具完成时的等待队列判断 | 回包构造位于 `AppState` 的实例方法中，随后会恢复原应用保存的 continuation。工具调用缓存则读写同一状态对象。Windows 有自己的请求和等待队列。[AppState](../Sources/CodeIsland/AppState.swift) · [工具调用缓存](../Sources/CodeIsland/AppState+ToolUseCache.swift) |
| `DesktopAppRouting`、`CodexMessages` | 只读等待、Codex 深链、线程状态与用户输入 | 这些方法分布在 `AppState` 扩展、界面模型和 `TerminalActivator` 中。线程处理依赖应用状态和 SwiftUI，终端激活依赖 AppKit 与 ApplicationServices。[Codex 扩展](../Sources/CodeIsland/AppState+CodexAppServer.swift) · [等待状态](../Sources/CodeIsland/AppState+DisplayOnlyWaits.swift) · [终端激活](../Sources/CodeIsland/TerminalActivator.swift) |
| `CardShortcut`、`CardOrdering`、`ApprovalAlwaysScope` | 卡片快捷键、排序、永久允许的作用域 | 快捷键操作会执行 SwiftUI 动画并修改卡片状态。排序与行视图度量在同一文件，作用域枚举直接声明在面板视图文件中。[快捷键](../Sources/CodeIsland/AppState+CardShortcuts.swift) · [排序](../Sources/CodeIsland/SessionListOrdering.swift) · [面板](../Sources/CodeIsland/NotchPanelView.swift) |
| `TrayDrawing`、`SoundTiming`、`PushStrings` | 托盘图形、静音时间和推送文案 | 绘制命令嵌在 AppKit 的图标回调里，声音管理器持有 `NSSound`，推送和本地化类引用 CoreGraphics 与 Combine。[托盘](../Sources/CodeIsland/StatusItemController.swift) · [声音](../Sources/CodeIsland/SoundManager.swift) · [推送](../Sources/CodeIsland/PushNotifier.swift) · [本地化](../Sources/CodeIsland/L10n.swift) |
| `InstallerCatalog`、`RemoteInstaller` | CLI 默认事件、安装条目、远程安装脚本 | 条目构造嵌在原安装器的配置读写方法中。远程 Python 函数位于 Swift 多行字符串内，完整入口还会加载应用资源并调用固定路径 `/usr/bin/ssh`。[本地安装器](../Sources/CodeIsland/ConfigInstaller.swift) · [远程安装器](../Sources/CodeIsland/RemoteInstaller.swift) |
| `ClaudeRecords`、`CodexRollout` | 会话记录、rollout 文件头及最近回合状态 | 同文件的会话发现流程调用 `ProcessTable`，依赖 macOS 的 libproc 和 sysctl。Windows 还需要通过自己的增量读取器处理未写完的最后一行。[Claude 发现](../Sources/CodeIsland/Discovery/ClaudeDiscovery.swift) · [Codex 发现](../Sources/CodeIsland/Discovery/CodexDiscovery.swift) · [进程表](../Sources/CodeIsland/ProcessTable.swift) |
| `CompanionState`、`CompanionFrames` | 伴侣设备的会话摘要和蓝牙分帧 | 摘要读取 `AppState` 的卡片和问题队列。分帧方法是 CoreBluetooth 外设类型的私有成员，Windows 的最大帧长由 WinRT 订阅者提供。[状态发布](../Sources/CodeIsland/ESP32StatePublisher.swift) · [蓝牙外设](../Sources/CodeIsland/AppleCompanionBluetoothPeripheral.swift) |
| `DiagnosticText` | 诊断信息脱敏 | 同文件的配置读取使用 `JSONSerialization.ReadingOptions.json5Allowed`。桌面 Swift 工具链缺少这个 Foundation 接口。[DiagnosticsRedaction](../Sources/CodeIsland/DiagnosticsRedaction.swift) |
| `JSONLTailer` | 日志记录与增量解析 | 原文件使用 Darwin 文件描述符和 `DispatchSource.makeFileSystemObjectSource` 监视文件。Windows 还接收已经解码的记录，而原解析函数的相应步骤是私有的。[JSONLTailer](../Sources/CodeIslandCore/JSONLTailer.swift) |
| `AiWorkStatusMapper` | Agentix 状态映射 | 映射器与 Unix socket 客户端在同一文件，连接和读写使用 `Darwin.connect/send/recv` 及 `SO_NOSIGPIPE`。[AiWorkWatchClient](../Sources/CodeIslandCore/AiWorkWatchClient.swift) |
| `ClaudeQuotaClient`、`PushTransport` | 配额请求、推送重试和重定向规则 | 原配额入口先访问 macOS Keychain，并使用 POSIX 信号管理子进程。推送传输设置 `waitsForConnectivity`，还需要保留请求级重定向代理的行为。[配额](../Sources/CodeIslandCore/ClaudeQuotaClient.swift) · [推送传输](../Sources/CodeIslandCore/PushTransport.swift) |

测试另有 3 个生成文件。`ClaudeQuotaTests` 的后两项需要 POSIX 子进程，`JSONLTailerTests` 的附加文件监视用例需要原版 watcher，`BridgeGoldenHarness` 的完整运行器需要原版桥接可执行文件、进程树和 Unix socket。三者的源码片段仍由同一个脚本生成。[测试生成规则](scripts/swift_sources.py)

## codeisland-ipc

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/client.rs](src-tauri/crates/codeisland-ipc/src/client.rs) | 命名管道连接、单次请求读写、连接与响应超时。 |
| [src/endpoint.rs](src-tauri/crates/codeisland-ipc/src/endpoint.rs) | Windows 用户 SID 对应的管道名、隔离实例的地址和 Unix 测试端点。 |
| [src/lib.rs](src-tauri/crates/codeisland-ipc/src/lib.rs) | 导出管道和帧接口。 |
| [src/protocol.rs](src-tauri/crates/codeisland-ipc/src/protocol.rs) | 保留已发布的 NDJSON 帧、请求 ID 和响应关联。新 hook 帧把提供方 JSON 交给 HookServer.intake，旧 Event 帧继续可读。 |
| [src/server.rs](src-tauri/crates/codeisland-ipc/src/server.rs) | Windows 管道接收循环。Unix socket 用于本地集成测试，连接上的事件交给同一接收器。 |
| [src/unix.rs](src-tauri/crates/codeisland-ipc/src/unix.rs) | Unix 测试端点的 UID 与连接。 |
| [src/windows.rs](src-tauri/crates/codeisland-ipc/src/windows.rs) | Win32 管道、访问控制列表和 SID。 |

## codeisland-hook

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/main.rs](src-tauri/crates/codeisland-hook/src/main.rs) | 本地 stdin 和父进程信息进入 HookIntake。保留 Win32 Toolhelp、WT/WezTerm 标识及 v1 升级握手。 |
| [tests/e2e.rs](src-tauri/crates/codeisland-hook/tests/e2e.rs) | 真正的 hook 子进程、管道、运行时审批和 stdout。 |

## codeisland-agents

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [build.rs](src-tauri/crates/codeisland-agents/build.rs) | 构建时从原版插件文件生成 Windows 传输版本。 |
| [src/codex_rules.rs](src-tauri/crates/codeisland-agents/src/codex_rules.rs) | CodexPermissionRules 生成、匹配和持久化规则。Rust 在解析用户决定后提供配置路径。 |
| [src/definitions.rs](src-tauri/crates/codeisland-agents/src/definitions.rs) | ConfigInstaller.builtInCLIs 和 defaultEvents 提供事件与超时。Windows 表提供本地程序、配置文件及历史 UI 标识。 |
| [src/doctor.rs](src-tauri/crates/codeisland-agents/src/doctor.rs) | 按 Windows 搜索路径检测程序和配置，并输出 CLI 状态。 |
| [src/install.rs](src-tauri/crates/codeisland-agents/src/install.rs) | ConfigInstaller 创建提供方 JSON 条目、Kimi 表、Trae 匹配器、MiniMax 清单及 OpenCode 引用。JSONMinimalEditor 保留其他顶层字段。Windows 管理标记、备份、原子写入和 Hermes YAML 注释保留按已有安装格式执行。 |
| [src/lib.rs](src-tauri/crates/codeisland-agents/src/lib.rs) | 安装器错误和 crate 公共入口。 |
| [src/normalize.rs](src-tauri/crates/codeisland-agents/src/normalize.rs) | HookIntake、HookEvent、EventNormalizer、QuestionPayload 和原版多问题解析。Rust 将结果投影到既有 AgentEvent，处理旧插件的数字 ID、子卡 ID 和终端字段。 |
| [src/paths.rs](src-tauri/crates/codeisland-agents/src/paths.rs) | ClaudeConfigPaths.resolve 选择配置根。盘符、UNC、PATHEXT、注册位置和命令行引号由 Windows 处理。 |
| [src/plugins.rs](src-tauri/crates/codeisland-agents/src/plugins.rs) | 构建时读取原版 OpenCode、Pi、OMP 和 OpenClaw 资源。MiMo 使用 ConfigInstaller.mimoPluginSource。Windows 提供子进程传输。 |
| [src/render.rs](src-tauri/crates/codeisland-agents/src/render.rs) | AppState 的审批、拒绝、会话规则、问题和跳过回包，Gemini 经过 HookIntake.stdout。保留旧版 plain 客户端输出。 |
| [tests/install_tests.rs](src-tauri/crates/codeisland-agents/tests/install_tests.rs) | 安装、卸载、保留用户配置及重复安装。 |
| [tests/normalize_tests.rs](src-tauri/crates/codeisland-agents/tests/normalize_tests.rs) | 提供方输入到桌面卡片与原始事件名。 |
| [tests/render_tests.rs](src-tauri/crates/codeisland-agents/tests/render_tests.rs) | 原生审批结果与各提供方 stdout。 |

## codeisland-domain

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/approval.rs](src-tauri/crates/codeisland-domain/src/approval.rs) | AppState.agentKeepsAlwaysRule 与 CodexPermissionRules。判断使用请求的原始来源。 |
| [src/attention.rs](src-tauri/crates/codeisland-domain/src/attention.rs) | AttentionPolicy、EventSoundRouting、SceneMuteState 与 SoundManager 使用 Windows 提供的前台、离开和静音事实。 |
| [src/claude_desktop_code.rs](src-tauri/crates/codeisland-domain/src/claude_desktop_code.rs) | ClaudeDesktopCodeSession 校验原始会话 ID。 |
| [src/cowork.rs](src-tauri/crates/codeisland-domain/src/cowork.rs) | CoworkSession、CoworkAuditParser 和 CoworkAuditState 解析元数据、审计事件、权限与深链。 |
| [src/discovery.rs](src-tauri/crates/codeisland-domain/src/discovery.rs) | 恢复数据的交换结构及 Windows 卷、大小写、扩展路径前缀的比较。 |
| [src/follow_up.rs](src-tauri/crates/codeisland-domain/src/follow_up.rs) | FollowUpReminderScheduler 管理提醒，用 Rust 时钟通知 Windows 运行时。 |
| [src/hook_tool.rs](src-tauri/crates/codeisland-domain/src/hook_tool.rs) | 原版工具事件处理更新工具名、活动说明和工具调用 ID。 |
| [src/lib.rs](src-tauri/crates/codeisland-domain/src/lib.rs) | crate 导出、已有 UI 类型绑定和 Codex Desktop 卡片标识。 |
| [src/ordering.rs](src-tauri/crates/codeisland-domain/src/ordering.rs) | SessionListOrdering 与 IdleSessionRemovalPolicy 提供排名及分组。 |
| [src/push_text.rs](src-tauri/crates/codeisland-domain/src/push_text.rs) | PushMessageFormatter 清理推送正文。 |
| [src/session_metadata.rs](src-tauri/crates/codeisland-domain/src/session_metadata.rs) | JSONLTailer 和 AgentTaskParsing 从历史记录、hook 与 Codex 计划中恢复消息和任务。 |
| [src/settings.rs](src-tauri/crates/codeisland-domain/src/settings.rs) | NotchAnimationSpeed、SoundManager 及声音曲线共用。Windows 配置序列化、旧字段迁移、设备和快捷键采用桌面格式。 |
| [src/store.rs](src-tauri/crates/codeisland-domain/src/store.rs) | reduceHookEvent、原版任务及等待状态驱动真实 hook 和有对应语义的日志事件。卡片索引、历史游标、等待者、旧 UI 子卡及恢复记录由桌面存储维护。 |
| [src/swift_session.rs](src-tauri/crates/codeisland-domain/src/swift_session.rs) | 把旧持久化记录和桌面事件交给原版 reducer，将状态、活动、工具、回复和完成效果写回。 |
| [src/types.rs](src-tauri/crates/codeisland-domain/src/types.rs) | 现有 JSON / TypeScript 契约。会话项目名取 SessionSnapshot.displayName，Windows 路径先转换分隔符。 |
| [tests/store_tests.rs](src-tauri/crates/codeisland-domain/tests/store_tests.rs) | 会话归并、复活、排队、迟到记录、任务和恢复。 |

## codeisland-runtime

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/codex_app_server.rs](src-tauri/crates/codeisland-runtime/src/codex_app_server.rs) | CodexAppServerClient、原版 Codex 问题与回复逻辑。保留 Windows 子进程标准流及请求关联。 |
| [src/codex_host.rs](src-tauri/crates/codeisland-runtime/src/codex_host.rs) | Windows 上定位 Codex 安装与 GUI 可执行文件。 |
| [src/cowork_host.rs](src-tauri/crates/codeisland-runtime/src/cowork_host.rs) | Windows Claude Desktop 的进程和本地数据目录。 |
| [src/cowork_watcher.rs](src-tauri/crates/codeisland-runtime/src/cowork_watcher.rs) | 监视本地目录、游标与轮询，使用 CoworkSession 与 CoworkAuditState 解释记录。 |
| [src/follow_up.rs](src-tauri/crates/codeisland-runtime/src/follow_up.rs) | 执行原版提醒调度器给出的动作，关联桌面队列及其当前请求。 |
| [src/hook_policy.rs](src-tauri/crates/codeisland-runtime/src/hook_policy.rs) | HookServer 原版的目录排除、来源审批和设置规则。 |
| [src/ipc_handler.rs](src-tauri/crates/codeisland-runtime/src/ipc_handler.rs) | 本地管道与 SSH 共用 HookServer.intake、routeKind 与远端目录策略。Rust 绑定连接来源、托管等待者和回应。 |
| [src/island.rs](src-tauri/crates/codeisland-runtime/src/island.rs) | Tauri 可见界面的卡片及请求 ID，对应原版 IslandSurface。 |
| [src/lib.rs](src-tauri/crates/codeisland-runtime/src/lib.rs) | 导出运行时服务。 |
| [src/persistence.rs](src-tauri/crates/codeisland-runtime/src/persistence.rs) | Windows 应用数据目录、原子写入、版本迁移和损坏文件恢复。 |
| [src/process.rs](src-tauri/crates/codeisland-runtime/src/process.rs) | 按 OS 查询进程是否存活。 |
| [src/provider_discovery.rs](src-tauri/crates/codeisland-runtime/src/provider_discovery.rs) | ClaudeDiscovery、CodexDiscovery 及原版 SQLite 查询读取记录。Windows 负责目录遍历、恢复游标与 WSL。 |
| [src/push_format.rs](src-tauri/crates/codeisland-runtime/src/push_format.rs) | 原版 PushMessageFormatter、PushNotifier 文案与推送隐私规则。 |
| [src/runtime.rs](src-tauri/crates/codeisland-runtime/src/runtime.rs) | 原版 session reducer、卡片快捷键、AttentionPolicy 和提醒策略。Rust 在锁内校验可见请求 ID，安排 Tauri 通知、定时器和响应通道。 |
| [src/tailer.rs](src-tauri/crates/codeisland-runtime/src/tailer.rs) | JSONLTailer 解读提示、回复、清单和公开输出。Rust 维护 Windows / WSL 文件位置、偏移、轮转及日志事件到桌面队列的映射。 |
| [src/wsl.rs](src-tauri/crates/codeisland-runtime/src/wsl.rs) | 通过注册表和 wsl.exe 枚举发行版、路径、用户与日志根。 |
| [src/wsl_process.rs](src-tauri/crates/codeisland-runtime/src/wsl_process.rs) | 用 Linux procfs 确认 WSL 进程身份、打开文件和终端。 |

## codeisland-integrations

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/apple_companion.rs](src-tauri/crates/codeisland-integrations/src/apple_companion.rs) | ESP32StatePublisher 的会话排序、消息预览、问题与 Apple Companion 状态。Bluetooth 分帧沿用原版。桌面隐藏的问题和秘密输入在投影前处理。 |
| [src/ble.rs](src-tauri/crates/codeisland-integrations/src/ble.rs) | ESP32Protocol 的帧和上行事件。设备发现、特征值、MTU 及订阅由 WinRT 负责。 |
| [src/claude_limits.rs](src-tauri/crates/codeisland-integrations/src/claude_limits.rs) | ClaudeQuotaClient 与 ClaudeQuotaScheduler 解释返回值并安排下次拉取。 |
| [src/hook_webhook.rs](src-tauri/crates/codeisland-integrations/src/hook_webhook.rs) | HookServer 的原版筛选和 JSON 信封，桌面使用其 HTTP 发送器。 |
| [src/lan.rs](src-tauri/crates/codeisland-integrations/src/lan.rs) | 此 Windows LAN 客户端使用六位配对码及自己的 HTTP 接口。令牌摘要调用 Swift CryptoKit。 |
| [src/lib.rs](src-tauri/crates/codeisland-integrations/src/lib.rs) | 集成模块导出和平台随机数来源。 |
| [src/push.rs](src-tauri/crates/codeisland-integrations/src/push.rs) | 新格式使用 PushRequestBuilder 和原版重试策略。旧版单目标配置继续按其 Bark GET、ntfy 请求头、Telegram/Lark 纯文本等已发布格式发送。 |
| [src/push_channels.rs](src-tauri/crates/codeisland-integrations/src/push_channels.rs) | PushRequestBuilder、PushEndpoint、PushResponseInterpreter。Discord 和通用 webhook 使用桌面扩展协议。 |
| [src/quota.rs](src-tauri/crates/codeisland-integrations/src/quota.rs) | ClaudeUsageScanner、模型标签和原版额度计算；Windows 提供账户目录。 |
| [src/ssh.rs](src-tauri/crates/codeisland-integrations/src/ssh.rs) | RemoteReconnectPolicy、RemoteInstaller 的 shell 引号、OpenCode 配置及远端安装器原始 Python 函数。Windows 使用 SSH/SFTP、每主机安装清单、锁和卸载。 |
| [src/ssh_config.rs](src-tauri/crates/codeisland-integrations/src/ssh_config.rs) | Windows OpenSSH 配置、IdentityAgent、密钥与跳板主机解析。 |

## codeisland-cli

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [src/main.rs](src-tauri/crates/codeisland-cli/src/main.rs) | Windows 命令行参数、安装、诊断和本地 hook 子进程入口。 |

## codeisland-swift

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [build.rs](src-tauri/crates/codeisland-swift/build.rs) | SwiftPM 暂存、依赖跟踪、编译和随包分发动态库。 |
| [src/lib.rs](src-tauri/crates/codeisland-swift/src/lib.rs) | Swift 动态库的装载、C ABI、返回缓冲区与持有状态的同步。 |

## 桌面应用

| 文件 | 共用的实现与 Windows 职责 |
| --- | --- |
| [build.rs](src-tauri/build.rs) | Tauri 原生构建入口。 |
| [src/apple_companion.rs](src-tauri/src/apple_companion.rs) | WinRT BLE 外设、订阅和发布循环；状态及数据帧取原版发布器。 |
| [src/away.rs](src-tauri/src/away.rs) | Win32 空闲时间、锁屏、屏保。 |
| [src/claude_quota.rs](src-tauri/src/claude_quota.rs) | 把原版额度调度器接到 Tauri 定时器。 |
| [src/commands.rs](src-tauri/src/commands.rs) | Tauri invoke、系统对话框和服务入口。动作交给各自的共享逻辑。 |
| [src/diagnostics.rs](src-tauri/src/diagnostics.rs) | DiagnosticsRedaction 清理结构化数据。Windows 另处理日志、用户路径和本地设置的秘密字段。 |
| [src/events.rs](src-tauri/src/events.rs) | 把运行时更新传给 Tauri WebView。 |
| [src/hover.rs](src-tauri/src/hover.rs) | Win32 光标、鼠标键、穿透与窗口位置。 |
| [src/lib.rs](src-tauri/src/lib.rs) | Tauri 生命周期、插件、窗口、托盘和自启动。 |
| [src/main.rs](src-tauri/src/main.rs) | 桌面进程入口。 |
| [src/remote_connections.rs](src-tauri/src/remote_connections.rs) | RemoteReconnectPolicy 决定重连动作。Rust 执行 SSH 生命周期及 Windows 唤醒、网络事件。 |
| [src/shortcuts.rs](src-tauri/src/shortcuts.rs) | Tauri 注册 Windows 热键，原版 performCardShortcut 决定显示或操作哪张卡。 |
| [src/sound_files.rs](src-tauri/src/sound_files.rs) | Windows 文件选择器和自定义声音文件读写。 |
| [src/ssh_signals.rs](src-tauri/src/ssh_signals.rs) | Win32 窗口及网络变化通知。 |
| [src/terminal.rs](src-tauri/src/terminal.rs) | Windows Terminal、WezTerm、WSL、前台窗口及终端激活。 |
| [src/tray.rs](src-tauri/src/tray.rs) | Tauri 托盘和 Windows 任务栏主题、头像素色与系统语言。 |
| [src/tray_icon.rs](src-tauri/src/tray_icon.rs) | StatusItemController.menuBarIcon 的原始绘图语句直接产生图形路径和透明区域。Windows 按任务栏颜色和像素密度生成 RGBA 位图。 |
| [src/wezterm.rs](src-tauri/src/wezterm.rs) | 在 Windows 通过 WezTerm CLI 确认 pane、工作目录及前台窗口。 |
| [src/window.rs](src-tauri/src/window.rs) | Windows WebView、HWND、显示器工作区、DPI 和窗口层级。 |
| [src/windows_terminal.rs](src-tauri/src/windows_terminal.rs) | Windows UI Automation 查询并激活对应标签页。 |
| [src/workspace.rs](src-tauri/src/workspace.rs) | Codex 深链使用 TerminalActivator 的原版验证；Windows Shell 打开 URI、目录和本机应用。 |
| [src/wsl_console.rs](src-tauri/src/wsl_console.rs) | WSL 终端设备路径、标题序列与控制台写入。 |
