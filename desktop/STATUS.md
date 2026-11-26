# CodeIsland Desktop — build status

## Verified

- Windows 11 x64 native build succeeds:
  `C:\Users\XXX\tools\tauri-msvc.bat build --debug --no-bundle`
  → `C:\cargo-target\codeisland\debug\codeisland-desktop.exe`.
- The Release NSIS build succeeds. Its desktop executable and both
  sidecars were checked against the bundled payload.
- **260 Rust tests pass** in the default workspace suite and **262 pass** with
  the SSH feature enabled. Three OpenSSH fixtures were run separately against
  an isolated, real SSH server.
- **78 frontend tests pass** and `npm run build` succeeds.
- Earlier live smoke covered approval auto-expansion, resolving a waiting hook
  with Allow, all 10 Settings pages, and the shadowless island at launch.
- Custom CLI smoke: added `E2E Tool` on the Hooks page, which wrote a
  Claude-shaped config with 10 events and the `codeisland-e2etool` marker,
  then removing the row uninstalled every managed hook from that file.
- Remote smoke against a live OpenSSH container: added a host, connected with
  key auth (host key trusted on first use), forwarded local 18080 to a remote
  HTTP server and got HTTP 200, then deployed the hook over SFTP
  (`/home/test/.codeisland/codeisland-hook`, executable).

## Earlier Windows hover and header smoke (2026-11-11)

- Screen sampling passed three cycles on the primary display and two on the
  secondary display at 150% DPI with usage stats enabled. The island stayed at
  1170 × 750 physical pixels (780 × 500 logical pixels). The visible panel
  reached full height 723–785 ms after pointer entry and folded 523–591 ms
  after pointer exit, including exit into the transparent part of the HWND.
- The folded window passed clicks to an underlying test window. Clicking the
  strip expanded it in 57 ms.
- The gear opened foreground Settings on the first click (65 ms) and restored
  the same HWND after minimization (39 ms). Mute flipped the persisted sound
  setting, and the power button exited on the second click.
- Win32 owns the hover transitions. Collapsed state is transient, and local
  Claude usage scans run on a blocking worker so the UI thread can keep
  processing hover and clicks.
- Windows debug build, 185 Rust tests, and 22 frontend tests passed.

## Windows spring and header acceptance (2026-11-11)

- At 150% scaling the primary display passed three hover cycles and the second
  display passed two. The HWND stayed at 1170 × 750 physical pixels throughout.
  The visible wing reached its full-width sample 839–865 ms after entry,
  including the configured 500 ms dwell. Intermediate samples advanced through
  the spring without a half-open hold. Leaving into either the transparent
  area or outside the HWND cleared the wing in 627–732 ms.
- A normal compact click and clicks in the folded transparent area reached
  the underlying test window. The first approval shortcut revealed the
  actual waiting hook and the second returned an approval to it.
- The gear focused Settings in 78 ms and restored the same HWND from
  minimization in 44 ms. Mute persisted and the second power click exited.
- The smoke ran on a dedicated Windows named pipe and temporary settings and
  sessions. Chromium separately verified 580 px panel width, 25 px header
  height, continuous mid-flight reversal and reduced motion.
- After the motion review, the quit confirmation lays out its full hit target
  once and moves neighboring controls with transforms. The idle strip keeps
  its 80 px hover reserve and animates the contour clip. Chromium verified
  both transitions, and the rebuilt Windows app passed Settings, mute and
  two-click quit in both the expanded panel and idle strip.

## Windows visual regression (2026-11-11)

- The expanded header uses the original gray notch mark and an outlined
  gear. The bundled mascot GIFs and their paused frames use the transparent
  renders from the Swift project's documentation. Chromium decoded all 14
  assets and checked the transparent frame corners.
- Comfortable cards prioritize the project name over the short session ID
  and use the terminal and optional model as secondary metadata. All three
  sample names fit at 580 px with the previous 13 px font and details enabled.
  The Swift appearance defaults apply to new or missing settings. Existing
  explicit font, line count, and metadata preferences carry forward.
- Idle hover moves the mascot and controls with the visible contour. The
  controls fade out and immediately stop accepting input on leave. Chromium
  verified the painted position and gear hit testing with normal and reduced
  motion through the same hover event that the Windows host emits.
- The final Windows build opened centered at 150% DPI with legacy coordinates
  present and dragging disabled. Three native open and close cycles passed.
  Click-through, approval reveal and resolution, Settings open and restore,
  persisted mute, and two-click quit passed. The idle strip passed Settings,
  mute, and quit as well. These checks used isolated temporary settings.

## Collapsed strip Swift parity (2026-11-11)

- The simulated non-notched strip follows the Swift 25 px top-bar fallback
  and one-line layout, with the Windows window meeting the top of the work
  area. The 7 px prehover width moves both wings with the contour and
  preserves the 6 px side padding. The tool-status setting swaps the width
  during the original curtain transition's hidden phase.
- The compact activity mirrors Swift precedence. A tool uses the original
  color categories, short description and masked shimmer. Claude tool input
  supplies the description, and a late tool-end event preserves the newer
  tool's details. Current-turn Codex output appears as one-line plain text
  when there is no tool. Historical replay and late output cannot revive
  the compact preview. Changing text uses Swift's short blur and text
  replacement timing. The project-name toggle keeps the folder hidden,
  and waiting badges retain their distinct glyphs and pulses.
- Clawd, Dex, Cursor and Gemini use the Swift sleeping, working and waiting
  scenes in the compact strip. The 50 ms clock grid, speed setting and
  animation gate control canvas updates. Chromium checked all 12 poses,
  static idle, 25 px geometry, tooltip values and the width change while
  the curtain was hidden.
- At 150% DPI the native strip touched the top work-area edge and painted
  37 physical pixels high. The folded tool fixture displayed
  `Bash cargo test --workspace`. The final Windows debug build passed three
  native hover cycles, click-through and approval shortcuts. Settings,
  persisted mute and two-click quit passed in both the panel and idle strip.
- The original 37 px custom height was also checked at 150% DPI. The folded
  contour painted 55 physical pixels, two hover cycles passed, and the gear
  opened and restored Settings in the session panel and idle strip.

## Windows Codex Desktop discovery (2026-11-11)

- The Windows host detector accepts both `Codex.exe` and `ChatGPT.exe`
  inside the `OpenAI.Codex_*` installation. The installed version uses
  `ChatGPT.exe` for its desktop host. Tests also cover standalone ChatGPT,
  the Codex CLI, VS Code's agent host and unrelated package executables.
- Tailer checkpoints exclude cursors that have not been initialized. A legacy
  zero checkpoint for a recent Codex Desktop rollout gets the bounded recovery
  scan. For an older rollout it starts at EOF. A regression test opens the
  Desktop gate and checks both cases and a subsequent live append.
- On a fresh Desktop attach, the newest lifecycle marker can be older than
  the 64 KiB preview. The bounded reverse scan restores the phase separately
  from the preview. It preserves actionable requests, terminal states, and
  newer provider activity. The regression fixture restores a saved snapshot
  and an EOF cursor for both active and settled long turns.
- The final Windows build was restarted with the normal user profile.
  The current real Codex thread appeared in the recovered state, and its
  rollout checkpoint advanced while the thread was still being written.
  The live compact strip showed one active session out of nine.

## Compact idle mascot (2026-11-11)

- Local Claude turns without an observable process settle in place after
  the Swift 180 s tool or 300 s thinking timeout. A stale transcript update
  is settled before publishing the UI. Replies stay on their cards, and
  live monitored, remote, Cowork and waiting requests remain with their own
  lifecycle. Runtime tests cover stale cleanup and old transcript recovery.
- The mascot gate uses the visible compact surface when the last card
  disappears. Chromium verified that an empty folded strip holds its pixels,
  including when the raw state still points to the list. A fixture with
  completed Codex cards and an idle Claude card also held still for 4.7 s.
- Windows confirmed the visible idle mascot pixels stayed identical over
  2.65 s at the user's 35 px geometry. Settings, mute and quit were checked
  on the isolated instance, then the rebuilt app was launched normally.

## Windows application icon (2026-11-11)

- The Tauri application and installer icons are generated from the original
  [rendered CodeIsland logo](../logo.png). The Windows ICO is an explicit
  build input so icon-only changes refresh the executable resource.
- About uses the 100 px Swift `AppLogoView` geometry. Its layout shares the
  mark with the expanded header. Chromium verified the About metrics and
  identical expanded-header pixels before and after the refactor.
- The Windows tray renders the Swift status item's 18-point capsule and two
  transparent eyes. It uses the system taskbar light/dark preference and
  updates on registry notification. The raster test checks both tints.
- The Windows debug build embeds all six updated ICO sizes (16, 24, 32, 48,
  64, and 256 px). The native smoke test retrieved the updated 32 px icon
  from both the island and Settings windows, opened and restored Settings
  with the gear, and verified mute and two-click quit. The rebuilt app was
  restarted and responded to Windows messages.
- The frontend build and all 60 frontend tests passed. The Docker Rust
  workspace regression passed with the desktop crate excluded.

## Session mascot movement (2026-11-11)

- On the old Windows build, WebView2 measured the same 2 px jump in a
  working row and two completed rows. All three played status-independent
  GIFs. The new renderer uses the Swift sleeping, working, and waiting
  scenes for Claude, Codex, Cursor, Gemini, OpenCode, and Qoder at the
  original 18 px and 32 px list sizes. The scenes share a 50 ms frame clock.
  The folded strip holds the sleeping frame when every visible session is
  idle. Providers whose Swift scenes still need porting use a static sprite.
- Retained Codex rollout headers supply the parent for worker and review
  threads, including at a saved EOF cursor. The change preserves their turn
  status. Completed Codex workers leave the list and compact rotation,
  following Swift's default separate mode. Active workers and children with
  an actionable Codex request keep their cards.
- Chromium verified fixed mascot bounds, per-session poses in both list
  densities, and stable canvases across repeated state snapshots. Windows
  WebView2 verified zero vertical offset in all three expanded rows, the
  working pose advancing, and identical folded idle pixels over 2.7 s.
- The rebuilt app was restarted with the normal profile and responded to
  Windows messages. Its saved state recovered 12 child-to-parent links.
  The current long-running Codex Desktop turn recovered as `running` after
  restart. The Rust regression covers both parent formats, saved EOF cursors,
  and lifecycle markers outside the preview window.

## Windows session card jumps (2026-11-11)

- The old build had a click handler only on Cowork session cards. A Windows
  WebView2 fixture reproduced that an ordinary card click left the matching
  minimized window untouched. Both list densities and the completion card
  now request activation for the clicked session ID. Links, request controls,
  and drag handles have independent actions.
- Win32 chooses the window matching the session workspace and checks that
  the selected window actually reached the foreground. Codex Desktop uses
  the selected thread's registered `codex://threads/<uuid>` URI and checks the
  foreground host against the same package identity as discovery. The
  auto-collapse setting applies only to a successful jump from the same
  list or completion card.
- The rebuilt Windows app restored and activated the selected minimized
  window from two windows in the same process. A missing target left the
  list open. The current Codex thread link foregrounded the installed
  Codex Desktop package. The normal app was restarted after the fixtures.
  All 69 frontend, 248 shared Rust, and six Windows desktop tests passed.
- Two existing Codex Desktop conversations were also tested with pointer
  clicks through the Windows WebView2 session cards. Codex's renderer logged
  the selected `/local/` route for `…3c0531` and then for `…f8319f` in the same
  window, 1.365 s and 0.884 s after the respective clicks. The second click
  returned to the conversation that was selected before the test.

## Windows Claude Code discovery and navigation (2026-11-12)

- Discovery validates the Windows FILETIME in Claude Code's session record
  against the live process creation time. It reads the record's `busy` and
  `idle` updates alongside the transcript, and checks the default, configured,
  and extra Claude roots.
- Claude Desktop Code uses the validated host session ID from discovery or
  the hook environment. The Windows protocol opens the corresponding
  `/epitaxy/<host-id>` route in the installed app, including its third-party
  configuration. Cowork keeps its own route. Standalone Claude Code uses its
  owning terminal.
- The Rust fixtures cover process identity, current status with an older
  transcript, config roots, host ID validation, and the separate navigation
  routes. All nine Windows desktop tests pass.
- The Windows Release app recovered a running card using a fixture record
  tied to a live, isolated native Claude CLI. Updating only the record to
  idle changed the card to idle with the transcript untouched. The fixture
  CLI received no prompts.
- Pointer clicks on two different CodeIsland cards selected two existing
  Claude Desktop conversations in the installed third-party profile.
  Claude's renderer reported each matching host session ID in sequence.
  The second click returned to the original conversation.
- Native Windows PID discovery excludes WSL PIDs. The WSL scanner keeps
  Linux process identities separately.

## Windows WSL transcript discovery (2026-11-12)

- Windows discovers Codex and Claude Code transcript roots for the default
  user of running WSL distributions. It resolves the default user and home
  from WSL registration and Linux user metadata, then checks the standard
  CLI locations. Discovery requires no extra directory setting.
- WSL reads run separately from the existing Windows transcript scan.
  Configured roots take precedence if they overlap an automatically
  discovered path. Initial recovery uses the same recent-file and bounded
  transcript-tail rules as the other providers.
- The 65 Windows runtime tests passed, including distribution-list decoding,
  default-user resolution, and overlapping UNC and drive paths.
- A separate Windows runtime on this machine recovered a recent real
  Ubuntu Claude CLI transcript, two synthetic transcripts from the default
  Ubuntu Codex and Claude directories, and a Windows Codex control.
  Transcript path overrides were unset. Both fixtures recovered as running
  and reflected appended replies and completion. The cold run took about
  25 seconds and the appended updates appeared in about 2.5 seconds.
- A second run started without the default Ubuntu Codex sessions directory.
  The running Windows runtime detected it on the next refresh after the
  fixture created it. Both runs removed their test transcripts and state.
- The Windows Release NSIS bundle was rebuilt and checked against the
  desktop executable and both sidecars.

## Windows WSL process state and terminal activation (2026-11-12)

- The WSL scanner associates Codex sessions with the process that holds the
  exact rollout open. Claude Code uses its live session record and the
  Linux process start time to validate the session and read busy/idle state.
  Active CLI processes can also supply their own configured transcript roots.
- A Windows Terminal jump revalidates the association, identifies the tab
  through its WSL pseudoterminal, and checks that Windows UI Automation reports
  that tab selected in the foreground window. Unverified jumps return failure.
- The Windows x64 Release NSIS bundle was rebuilt for these changes. The
  extracted desktop executable, hook, and CLI match the Release outputs.
  End-to-end verification of switching between the live Codex and Claude Code
  tabs is still pending.

## Windows launch audio and Codex sub-sessions (2026-11-12)

- The previous Release build dropped its boot cue when WebView2 held the
  island renderer before its first script for 1.5 seconds. The main renderer
  now requests the cue once after loading preferences and subscribing.
  Launch at login remains silent. The audio switches, custom files,
  quiet hours, and away checks still apply.
- Windows Debug and Release playback completed exactly once under the same
  delay. Login launch and disabling the boot cue stayed silent in Debug.
  The manual fixture also played the two configured Claude hook sounds
  from the bundled WAV files after receiving the sidecar events.
- Live Cowork start, prompt, and approval sounds now use the same event
  identifiers as the corresponding per-event preferences.
- Codex worker cards use the nickname or role from the rollout when the
  provider has no title. Separate mode shows active workers without adding
  their cards or settled workers to the parent's badge.
- Settings → Behavior → Codex Sub-Sessions offers separate, merge, and
  hide. Merge folds active descendants into the visible ancestor. Workers
  awaiting an answer or approval keep their own card.
- The Windows WebView2 fixture injected a Codex parent, child, and
  grandchild. Debug and Release both verified the Settings switch,
  persistence, the visible cards, and parent badge counts of zero, two,
  and zero. All nine Windows desktop Rust tests passed.

## Settings persistence and live behavior (2026-11-11)

- The previous Windows build lost two edits when Settings was closed
  immediately. The Settings webview now submits field patches on change,
  queues subsequent edits, and stays alive while hidden. The runtime saves
  one authoritative settings file before publishing, and the desktop and CLI
  read it with a legacy fallback. Failed writes preserve live values and
  the Settings page exposes retry.
- Monitor choices use the same stable identity as Win32 placement. Auto and
  cursor modes recalculate as their target changes. Horizontal dragging
  persists the final offset on release. Hover timers use the current delay
  and cancel pending collapse when that setting is disabled.
- Live consumers now apply the changed compact rotation interval, tool
  visibility in compact rows, and unlimited reply lines. The Swift history
  choices control the retained Rust event history. The unused theme and
  separate automatic-cleanup controls were removed from Settings. Existing
  serialized fields still load and the Swift cleanup duration controls expiry.
- Claude away summaries and structured task records, plus Codex plan
  records, populate recap and task displays. Legacy push channels now honor
  the same away filter as configured channels. Shortcut conflicts and hook
  installation failures surface in the interface.
- On the rebuilt Windows app, two simultaneous edits survived immediate
  close, reached the island, and were present on disk when Settings reopened.
  With the file locked, the island kept its old values and the Settings retry
  succeeded after unlocking. Selecting primary, secondary, then primary at
  150% DPI moved the native window to the expected physical monitor each time.
  Six Windows desktop tests also passed and the normal app was restarted.
- The rebuilt app again switched between both real Codex conversations,
  confirmed from the renderer route. The synthetic terminal fixture restored
  the test window but Windows withheld foreground focus in that run.

## macOS source parity

The Swift code in this worktree defines behavior and visual metrics. The
Windows shell uses Tauri and Win32 for window placement, activation, input
and transport.

| Swift source | Windows implementation and verification |
| --- | --- |
| [IslandSurface.swift](../Sources/CodeIsland/IslandSurface.swift), [AppState.swift](../Sources/CodeIsland/AppState.swift), [AppState+CardShortcuts.swift](../Sources/CodeIsland/AppState+CardShortcuts.swift) | [Runtime surfaces](src-tauri/crates/codeisland-runtime/src/island.rs) and [queue handling](src-tauri/crates/codeisland-runtime/src/runtime.rs) own the five surfaces and request identity. [Windows shortcuts](src-tauri/src/shortcuts.rs) reveal the target before acting and require acknowledgment from the rendered card. |
| [NotchPanelView.swift](../Sources/CodeIsland/NotchPanelView.swift), [ScreenDetector.swift](../Sources/CodeIsland/ScreenDetector.swift), [NotchAnimation.swift](../Sources/CodeIsland/NotchAnimation.swift), [NotchAnimationMetrics.swift](../Sources/CodeIsland/NotchAnimationMetrics.swift) | [IslandSurface.tsx](src/components/IslandSurface.tsx) derives the simulated notch and carries spring position and velocity across reversals. [Win32 hover](src-tauri/src/hover.rs) distinguishes idle, list, and request-card behavior. Chromium verified the contour, the 580 px panel and 25 px header in the non-notched fixture, interrupted motion, and reduced motion. |
| [SessionListOrdering.swift](../Sources/CodeIsland/SessionListOrdering.swift), [QuitConfirmation.swift](../Sources/CodeIsland/QuitConfirmation.swift) | [ExpandedPanel.tsx](src/components/ExpandedPanel.tsx), [RequestCard.tsx](src/components/RequestCard.tsx) and [IslandHeaderControls.tsx](src/components/IslandHeaderControls.tsx) render independent cards, keep the list scrollable, and show and cancel the timed quit confirmation. Browser tests measured 34 px compact rows, 22 px icons with 24 px hit targets, and a separately scrolling command detail. |
| [IdleSessionRemoval.swift](../Sources/CodeIsland/IdleSessionRemoval.swift), [CompactBarRotation.swift](../Sources/CodeIsland/CompactBarRotation.swift) | [Session store](src-tauri/crates/codeisland-domain/src/store.rs), [idle confirmation](src/components/IdleSessionRemovalBanner.tsx) and [compact bar](src/components/CollapsedStrip.tsx) revalidate captured idle families as a group and rotate active sessions with the original status priority and mascot crossfade. Chromium verified the confirmation layout and marks. |
| [FollowUpReminderController.swift](../Sources/CodeIsland/FollowUpReminderController.swift), [FollowUpReminderScheduler.swift](../Sources/CodeIslandCore/FollowUpReminderScheduler.swift) | [Runtime scheduler](src-tauri/crates/codeisland-runtime/src/follow_up.rs) retries approvals and questions up to three times and unread completions once. Tests cover quiet/away catch-up, replacing a question, reading a completion, and jumping to a session. [Win32 away state](src-tauri/src/away.rs) reads the Windows lock, screensaver and display-power signals. |
| [NotchPanelView.swift](../Sources/CodeIsland/NotchPanelView.swift), [CodexPermissionRules.swift](../Sources/CodeIsland/CodexPermissionRules.swift) | [Approval card](src/components/ApprovalCard.tsx) renders structured command and edit details. [Rule handling](src-tauri/crates/codeisland-agents/src/codex_rules.rs) persists the frozen command prefix. Tests cover rules for unrelated commands, queued approvals and stale jump callbacks. |
| [Settings.swift](../Sources/CodeIsland/Settings.swift), [SettingsView.swift](../Sources/CodeIsland/SettingsView.swift), [HookServer.swift](../Sources/CodeIsland/HookServer.swift) | [Rust settings](src-tauri/crates/codeisland-domain/src/settings.rs), [Appearance](src/settings/tabs/AppearanceTab.tsx), [Behavior](src/settings/tabs/BehaviorTab.tsx), and [hook policy](src-tauri/crates/codeisland-runtime/src/hook_policy.rs) cover spring speed, notch geometry, directory exclusions, webhook forwarding, and opt-in auto-approval. Historical settings deserialize with defaults. |
| [SSHForwarder.swift](../Sources/CodeIsland/SSHForwarder.swift), [RemoteInstaller.swift](../Sources/CodeIsland/RemoteInstaller.swift) | [SSH integration](src-tauri/crates/codeisland-integrations/src/ssh.rs) and [connection lifecycle](src-tauri/src/remote_connections.rs) cover a host-scoped reverse bridge, deploy/uninstall for seven original CLI schemas and two custom formats, startup auto-connect and retry. A real OpenSSH fixture verified repeat deployment, approval responses, uninstall ownership, and reconnect with the original forward port still serving HTTP. |
| [RemoteHost.swift](../Sources/CodeIsland/RemoteHost.swift), [SSHForwarder.swift](../Sources/CodeIsland/SSHForwarder.swift) | [OpenSSH configuration](src-tauri/crates/codeisland-integrations/src/ssh_config.rs) resolves aliases, identity files, agent sockets and ProxyJump. Separate real OpenSSH fixtures verified key-agent and jump authentication. [Windows resume and network signals](src-tauri/src/ssh_signals.rs) feed the reconnect lifecycle. |
| [AppState+CodexAppServer.swift](../Sources/CodeIsland/AppState+CodexAppServer.swift), [CodexAppServerClient.swift](../Sources/CodeIslandCore/CodexAppServerClient.swift) | [App-server observer](src-tauri/crates/codeisland-runtime/src/codex_app_server.rs) handles stdio handshake, thread state, user input replies and disconnect cleanup. Desktop has a separate session namespace and rollout recovery. A subprocess fixture exercised the protocol and verified that Desktop events leave same-ID CLI sessions alone. |
| [CodexDiscovery.swift](../Sources/CodeIsland/Discovery/CodexDiscovery.swift), [ClaudeDiscovery.swift](../Sources/CodeIsland/Discovery/ClaudeDiscovery.swift) | [Provider discovery](src-tauri/crates/codeisland-runtime/src/provider_discovery.rs) reads recent Codex Desktop SQLite state and matches native Claude session records to live processes. Recovery merges with hook and transcript cards and keeps live request state. The Windows database matched the expected schema, and recoverable positive cases used fixtures. |
| [CoworkSessionWatcher.swift](../Sources/CodeIsland/CoworkSessionWatcher.swift), [CoworkAuditLog.swift](../Sources/CodeIslandCore/CoworkAuditLog.swift) | [Cowork observer](src-tauri/crates/codeisland-runtime/src/cowork_watcher.rs) recovers local Cowork and Chat cards, parses audit changes, and offers a Claude Desktop deep link for requests answered in Claude. The Windows hashed session-directory layout was checked read-only against two actual local records, in addition to synthetic fixtures. |
| [ClaudeQuotaMonitor.swift](../Sources/CodeIsland/ClaudeQuotaMonitor.swift), [ClaudeUsageScanner.swift](../Sources/CodeIslandCore/ClaudeUsageScanner.swift) | [Plan limit client](src-tauri/crates/codeisland-integrations/src/claude_limits.rs), [refresh policy](src-tauri/src/claude_quota.rs) and [usage scanner](src-tauri/crates/codeisland-integrations/src/quota.rs) drive the [chip and footer](src/components/ClaudeQuota.tsx). Tests cover both quota response schemas, debouncing, throttling, error backoff and partial local transcript lines. |
| [TerminalVisibilityDetector.swift](../Sources/CodeIsland/TerminalVisibilityDetector.swift), [TerminalActivator.swift](../Sources/CodeIsland/TerminalActivator.swift) | [Win32 terminal probe](src-tauri/src/terminal.rs) matches the foreground process and its terminal or IDE host. It separates application-level Smart Suppress from verified session visibility, and the approval jump only closes the card after a successful focus. [WezTerm integration](src-tauri/src/wezterm.rs) captures pane IDs and uses the original pane-list and activation commands. |
| [PushSettingsView.swift](../Sources/CodeIsland/PushSettingsView.swift), [PushChannel.swift](../Sources/CodeIslandCore/PushChannel.swift), [PushTransport.swift](../Sources/CodeIslandCore/PushTransport.swift) | [Push preferences](src/settings/tabs/PushSettings.tsx), [channel renderers](src-tauri/crates/codeisland-integrations/src/push_channels.rs) and [transport](src-tauri/crates/codeisland-integrations/src/push.rs) cover the seven original services, channel event selection, privacy, signing and provider response checks. Local tests verify payloads and preserving POST through redirects. |
| [SoundManager.swift](../Sources/CodeIsland/SoundManager.swift), [SettingsView.swift](../Sources/CodeIsland/SettingsView.swift) | [Sound settings](src/settings/tabs/SoundTab.tsx) and [native file access](src-tauri/src/sound_files.rs) provide per-event custom audio, preview and reset. Chromium played both the selected WAV and the restored bundled WAV. |
| [MascotAnimationGate.swift](../Sources/CodeIsland/MascotAnimationGate.swift), [FrameClock.swift](../Sources/CodeIsland/FrameClock.swift) | [Island animation gating](src/components/IslandSurface.tsx) holds a single status pose while folded and idle, hidden, asleep, or using reduced motion. [Mascot canvases](src/components/Mascot.tsx) share one frame clock. |
| [AppleCompanionBluetoothPeripheral.swift](../Sources/CodeIsland/AppleCompanionBluetoothPeripheral.swift), [AppleCompanionPayload.swift](../Sources/CodeIslandCore/AppleCompanionPayload.swift) | [Windows GATT peripheral](src-tauri/src/apple_companion.rs) publishes the original read-only characteristic, status payload, and heartbeat. A standalone WinRT probe started advertising on this Windows adapter and the Rust tests verified the payload and fragmentation. |

## Open parity work

- Status-specific Swift mascot drawing for the providers beyond Claude,
  Codex, Cursor, Gemini, OpenCode, and Qoder. Those providers use a static
  fallback sprite.

- [Provider discovery and merge](../Sources/CodeIsland/Discovery/SessionDiscovery.swift)
  for the other CLIs, Codex CLI working-directory process matching, and
  legacy Claude discovery without a current native session record.
- Exact active terminal tab and multiplexer pane routing beyond WezTerm and
  the current foreground process and IDE checks.
- The [Apple companion peer-session features](../Sources/CodeIsland/AppleCompanionPublisher.swift)
  beyond read-only BLE status, including the transcript stream and remote
  actions that use Apple's Multipeer Connectivity transport.
- SSH custom ProxyCommand and setup for other remote users. The first
  remote hook install uses the explicit Deploy action.
- Tool-history parity and Cursor sub-session display modes, the original
  ESP32 device controls, and installer-backed update delivery.
- Native acceptance for Codex Desktop live questions and deep links, Cowork
  live updates and deep links, actual Anthropic subscription limits,
  WezTerm pane switching, Windows OpenSSH agent and wake/network events,
  native sound file selection and output, live push service accounts, and
  pairing and receiving updates on
  an iPhone. The Codex fixture exercised
  the child process owned by CodeIsland. The SSH reconnect fixture injected
  wake and network signals into a controller using a real SSH server.

## Audit fixes (macOS vs rewrite)

### Session / runtime
- Subagent events are isolated from the parent (`agent_id`, child session ids);
  SubagentStop never ends the parent.
- Interrupted turns (`stop_reason` user/interrupted/aborted) become Cancelled and
  stay inert until a new prompt; tool events no longer overwrite waiting cards.
- `SessionEnd` removes the card and drains its waiters; it is silent.
- Failed tool steps (`PostToolUseFailure`, `PermissionDenied`) are ToolEnd, not a
  failed turn; compaction is a silent heartbeat.
- Restore resets every card to Idle and clears transient/always-allow state.
- SessionStart rebuilds the card (keeps tasks) and lands Idle.
- Visible-session limit no longer deletes sessions; expiry only sweeps resting
  cards and `cleanup=0` means never.
- Always allow reuses the request's captured scope. Codex persists the selected
  command prefixes and later covers requests within those prefixes.
- Approval bursts ring once; completion reply text is captured.
- Push notifications are wired to the runtime; Smart Suppress state comes from
  the foreground watcher.

### Transcript recovery
- Codex + Claude + Cursor/Qoder/CodeBuddy tailers; launch scans only the last
  64 KB of live transcripts, never replays old history.
- Codex coverage: turn_context model/effort, user_message, turn_failed,
  response_item message, plan ignore, task/user/agent events.
- Stable event ids, raw-byte partial lines, shrink re-read, offset pruning.

### Hooks / adapters
- Codex `config.toml` gets `[features]` with `hooks = true`; ZCode uses the Claude schema; Qwen
  timeouts stay milliseconds; Copilot flat config; MiniMax plugin manifest;
  Hermes/Trae CLI YAML merge; extra agents (omp, google-antigravity, aiwork-cli,
  traecli-next); real OpenCode/Pi/OpenClaw plugin sources; `_ppid` fallback.
- Cursor gates are non-blocking; Antigravity prints `{"decision":"ask"}`;
  Claude AskUserQuestion answers are keyed by question header.
- Extra config roots and user-defined CLIs are installed alongside the
  built-ins; a custom CLI gets its own source marker and config key, and the
  per-CLI toggle only touches that CLI's entries.
- Tool-use correlation keeps the tool name/command a permission gate omits;
  Cursor digests dedupe replayed tool calls.
- Claude hooks use the macOS wildcard matcher; Codex/Gemini-style nested
  entries omit `matcher`, and best-effort JSON keeps its empty matcher.
- Installers follow symlinked configs and write the target, leaving the link
  in place (covered by a Unix test).
- OpenClaw's plugin opens a blocking PermissionRequest for dangerous shell
  commands and returns `{ block, blockReason }` when the island denies; its
  lanes stay stable across OpenClaw session resets.

### Remote hosts
- Settings connects to an SSH host, forwards local TCP traffic, and deploys
  the original Python remote hook plus managed CLI configuration over SFTP.
  Host keys are recorded on first use and verified on reconnect.

### Window / UI
- Clicking the island can focus it (typing answers works); show still never
  steals focus.
- Display choice (auto/cursor/pinned), monitor topology watcher, fullscreen
  hiding, outside-click collapse, and tray/single-instance Settings.
- The HWND stays at 780 × 500 logical pixels. The visible notch contour follows
  the original open and close spring responses. Win32 measures the configurable
  enter delay (500 ms by default) and the 500 ms list leave delay.
- Win32 controls island visibility and the folded window's click-through styles.
  This keeps the window visible when hover changes those styles.
- The gear queues Settings creation on the UI thread after the WebView2 click
  callback returns. Reopening Settings restores the minimized window before
  focusing it.
- Settings: 10 pages with the macOS layout; content/behavior/sound/mascots/
  remote/Buddy fields; 8 languages with English fallback.
- Island matches the macOS metrics: black shell, white-5% cards, 3 px status
  rail, 10 px radius, 14/10 padding, 32 px mascots, system monospace text,
  10 px status chips, 9.5 px tags, pixel ALL/STATUS/AGENT tabs, 22 px round
  icons, dashed divider, Always/Hide/Deny/Allow order.
- The Codex display mode controls standalone worker cards and the parent's
  `+N Sub` count. Other providers keep their existing folding rules.
- The six original WAV cues play from the island window. Error sounds are
  deduplicated per session.
