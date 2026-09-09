# codex-gui

`codex-gui` is a native Codex desktop client built with Zed's GPUI. It embeds the Codex app server in-process and renders its protocol state directly; there is no Electron layer or separate web frontend.

## Architecture

The main data flow is:

```text
Codex app-server requests and notifications
                    ↓
              AppServerBridge
                    ↓
                 CodexGui
                    ↓
     WorkspaceState + WindowState entities
                    ↓
     Sidebar / ChatPanel / ChatHistory views
```

- `src/main.rs` initializes GPUI, the Tokio runtime, assets, the root window, and the embedded app-server bridge.
- `src/bridge.rs` owns the in-process app-server client. It translates typed requests/responses and forwards server notifications through `BridgeEvent`.
- `src/app.rs` defines `CodexGui`, the application coordinator and owner of shared entities and top-level views.
- `src/app/` splits application behavior by responsibility:
  - `actions.rs`: user intents and outgoing app-server operations.
  - `effects.rs`: startup and other asynchronous workflows.
  - `results.rs`: RPC result handling.
  - `event_handler.rs`: app-server notification handling and canonical state mutation.
  - `thread_mapping.rs`: construction of GUI chat/project state from protocol threads.
- `src/gui/state.rs` contains shared application and UI state.
- `src/gui/chat_panel.rs` owns the main conversation surface, composer, thread controls, and settings controls.
- `src/gui/sidebar/` owns the virtualized project/thread navigation list and its row components.
- `src/gui/chat_history/` owns conversation projection and rendering.
- `src/gui/side_chat.rs` is currently a lightweight placeholder for temporary side-chat behavior.

## Conversation state

The app-server protocol's `Thread` is the canonical local representation of conversation content. Do not introduce a second list that copies user text, assistant Markdown, tool output, item kind, or ordering.

`ChatState` adds only state that the protocol model does not provide:

- transient per-item lifecycle from `ItemStarted` and `ItemCompleted` notifications;
- local notices that are not app-server `ThreadItem`s;
- an item-location index used to apply streaming deltas efficiently.
- thread-owned runtime state such as the active turn, loading/creation lifecycle, composer draft,
  edit session, and effective thread settings.

View choices such as expanded turns and expanded tool groups belong to `ChatHistory`, not `Thread` or `MessageState`.

When handling app-server events:

1. Mutate the relevant protocol `Thread`, `Turn`, or `ThreadItem` exactly once.
2. Update only the thin supplemental state required by the event envelope.
3. Call `cx.notify()` on the owning GPUI entity.
4. Let subscribed views rebuild their derived presentation.

Loaded threads and live threads must converge on the same `ChatState` representation. Account for `Thread.turns` being empty on responses that do not include turns and for `Turn.items_view` being partial.

## Sidebar list state

The sidebar is a lazy projection of canonical `WorkspaceState` plus sidebar-owned view state such as collapsed projects and pagination limits:

```text
WorkspaceState + sidebar view state
              ↓
   SidebarRowDisplayStatus
              ↓ row_at(index)
         SidebarRow
              ↓
       GPUI list renderer
```

- `SidebarRowDisplayStatus` is constant-size layout/index metadata, not a materialized row collection. Keep `row_at(index)` lazy; do not rebuild a `Vec<SidebarRow>` during render or pagination.
- `ListState` stores virtual slots, measurements, focus metadata, and scroll position; it does not own sidebar data. Keep `ListState::item_count()` equal to `SidebarRowDisplayStatus::len()`.
- Structural changes must go through the sidebar's explicit `insert_rows`, `remove_rows`, or `replace_rows` operations, which use `ListState::splice`. Do not reset `ListState` from `Render` when the item count changes.
- Pagination inserts newly exposed slots before the existing pager, or replaces the final pager with those slots. Folding and expansion insert or remove only the active project's child range.
- Model-driven notifications reconcile constant-size display metadata and apply structural operations without scanning all visible rows. Observe the active `ProjectState` as well as `WorkspaceState`, because its chat count contributes to the projection.
- `SidebarRow` may carry generic project/chat values; the production renderer specializes them to `Entity<ProjectState>` and `Entity<ChatState>` so each projected row is independently renderable.

## Chat history rendering

Chat history uses this pipeline:

```text
Thread + item lifecycle + history view state
                        ↓
                 projection.rs
                        ↓
TranscriptSnapshot { markdown, blocks }
                        ↓
               TextViewState / TextView
```

- `projection.rs` walks protocol turns/items directly, applies completed-turn folding and consecutive-tool grouping, and produces the visible transcript. Do not add an intermediate row model unless another renderer genuinely consumes it.
- `transcript.rs` owns `TranscriptSnapshot`, stable block markers, the block store, and the custom Markdown plugin.
- `view.rs` owns subscriptions, folding/expansion state, transcript synchronization, and the top-level `TextView`.
- `blocks/` owns custom GPUI content embedded in the transcript:
  - `messages.rs`: user bubbles and assistant headers;
  - `tools.rs`: tool grouping, summaries, expansion, and tool presentation;
  - `worked_summary.rs`: completed-turn summaries;
  - `mod.rs`: block identity and render dispatch.
- `math.rs` is the RaTeX Markdown extension used by the transcript.

Assistant bodies remain literal Markdown in `TranscriptSnapshot.markdown`. User bubbles, assistant headers, tool groups, and worked summaries are represented by stable `<CodexTranscriptBlock ... />` markers; their live render data is stored in `TranscriptSnapshot.blocks`.

Block IDs must depend only on semantic identity, never dynamic presentation state. Changing labels, tool status, or expansion must update the block store without changing the marker string.

Transcript synchronization follows these rules:

- replace the block store on every projection;
- when Markdown is unchanged, do not replace its text; after changing plugin-backed block data,
  call `TextViewState::remeasure_custom_block` when the changed block identity is known so only
  that row's cached height is invalidated without resetting the logical viewport; use
  `remeasure_content` only when the changed block cannot be identified;
- use `push_str` when the new Markdown is a strict append to the same chat;
- use `set_text` for chat switches, folding changes, or any earlier-content change.
- Configure transcript `TextViewState` with `FollowMode::Tail`. Tail following keeps streaming output
  visible, disengages when the user scrolls upward, and re-engages when they return to the end.

## GPUI state and event conventions

- Long-lived shared state is stored in `Entity<T>` and observed with `cx.observe` or `cx.subscribe_in`.
- Mutate an entity through `update`, then call `cx.notify()` when its rendered output may have changed.
- Keep cross-thread application state in `WorkspaceState`, thread-owned protocol/runtime state in `ChatState`, and window presentation state in `WindowState`. Pure widget machinery remains in the owning view.
- UI components call application intents through the parent `WeakEntity<CodexGui>` instead of talking to the bridge directly.
- Spawn asynchronous bridge work with `cx.spawn`, then apply results back on the GPUI thread with `this.update`.
- Preserve stable protocol IDs through projections instead of using list positions as identity.
- Prefer deriving grouping, status, and display labels from protocol fields over caching duplicate presentation data.

## Extending app-server support

For a new request-driven feature:

1. Add the typed bridge operation in `src/bridge.rs`.
2. Trigger it from an application intent in `src/app/actions.rs` or a workflow in `src/app/effects.rs`.
3. Handle the result in `src/app/results.rs`.
4. Keep bridge/RPC errors flowing through the existing error-to-notice path.

For a new notification or streamed item:

1. Handle it in `src/app/event_handler.rs`.
2. Update the canonical protocol object in `ChatState`.
3. Add only genuinely missing runtime metadata to `MessageState` or another narrowly scoped supplemental type.
4. Extend `projection.rs` and `blocks/` if the item should appear in chat history.

## Performance and diagnostics

- Transcript streaming should preserve the strict-append fast path; avoid changing earlier marker text for dynamic block updates.
- The sidebar uses a uniform-height virtual list. Keep row spacing inside the measured row box and update `ListState` when row count changes.
- `src/bin/text_view_scroll_profile.rs` contains transcript scrolling/performance scenarios.
- Runtime logs use `tracing`; configure verbosity through `RUST_LOG`.

## Dev environment

Manage environment and packaging with nix and cranelib.

## Code style

- Keep code clean and maintainable. Split into files or modules when necessary.
- When a possible refactor can substantially simplify code or make it easier to read and maintain, actively suggest it.

## Dev behavior

- Run quickly. Do NOT run `cargo test` and `cargo clippy` after your code changes. `cargo check` is enough.
