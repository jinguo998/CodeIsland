use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use codeisland_integrations::claude_limits::{self, FetchError, Scheduler, Snapshot};
use codeisland_runtime::{Runtime, RuntimeUpdate};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::task::JoinHandle;

type FetchTask = JoinHandle<Result<Snapshot, FetchError>>;

#[derive(Clone, Default, Serialize)]
pub struct PlanQuotaState {
    snapshot: Option<Snapshot>,
    error: Option<FetchError>,
}

#[derive(Default)]
pub struct PlanQuota(Mutex<PlanQuotaState>);

impl PlanQuota {
    fn set(&self, app: &AppHandle, update: impl FnOnce(&mut PlanQuotaState)) {
        let state = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            update(&mut state);
            state.clone()
        };
        let _ = app.emit("island://claude-quota", state);
    }
}

#[tauri::command]
pub fn claude_plan_quota(state: State<'_, PlanQuota>) -> PlanQuotaState {
    state
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub fn spawn(app: AppHandle, runtime: Arc<Runtime>) {
    if crate::is_isolated() {
        return;
    }

    let mut updates = runtime.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut scheduler = Scheduler::default();
        let mut in_flight: Option<FetchTask> = None;
        let mut was_enabled = false;
        let mut previous_surface = None;

        loop {
            let settings = runtime.settings();
            let surface = runtime.surface();
            let expanded = !surface.is_collapsed();
            let surface_changed = previous_surface.as_ref() != Some(&surface);
            previous_surface = Some(surface);

            if was_enabled && !settings.show_claude_quota {
                if let Some(task) = in_flight.take() {
                    task.abort();
                }
                scheduler = Scheduler::default();
                app.state::<PlanQuota>()
                    .set(&app, |state| *state = PlanQuotaState::default());
            }
            was_enabled = settings.show_claude_quota;

            let now = Instant::now();
            let can_fetch =
                settings.show_claude_quota && in_flight.is_none() && !runtime.is_display_asleep();
            let deadline =
                if can_fetch && expanded && surface_changed && scheduler.due_on_expand(now) {
                    Some(now)
                } else if can_fetch && (expanded || settings.claude_quota_chip != "off") {
                    scheduler.next_fire(now)
                } else {
                    None
                };
            if deadline.is_some_and(|deadline| deadline <= now) {
                let paths = codeisland_agents::paths::Paths::current().with_settings(&settings);
                let claude_dir = paths.claude_config_dir();
                scheduler.record_start(now);
                in_flight = Some(tokio::spawn(async move {
                    claude_limits::fetch(&claude_dir).await
                }));
                continue;
            }

            tokio::select! {
                biased;
                result = finish_fetch(&mut in_flight) => {
                    in_flight = None;
                    if runtime.settings().show_claude_quota {
                        scheduler.record_result(result.as_ref().err());
                        app.state::<PlanQuota>().set(&app, |state| match result {
                            Ok(snapshot) => {
                                state.snapshot = Some(snapshot);
                                state.error = None;
                            }
                            Err(error) => state.error = Some(error),
                        });
                    }
                }
                _ = wait_until(deadline) => {}
                update = updates.recv() => match update {
                    Ok(RuntimeUpdate::LocalClaudeStop) if settings.show_claude_quota => {
                        scheduler.record_stop(Instant::now());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
            }
        }
    });
}

async fn finish_fetch(task: &mut Option<FetchTask>) -> Result<Snapshot, FetchError> {
    match task {
        Some(task) => task.await.unwrap_or(Err(FetchError::Transport)),
        None => pending().await,
    }
}

async fn wait_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => pending().await,
    }
}
