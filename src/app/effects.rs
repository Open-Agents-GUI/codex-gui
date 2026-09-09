//! Start application-level bridge workflows.

use super::CodexGui;
use crate::workspace::workspace_path;
use gpui::Context;

impl CodexGui {
    pub(super) fn load_startup_data(&self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        cx.spawn(async move |this, cx| {
            let models_result = bridge.list_models().await;
            let settings_result = bridge.read_chat_settings().await;
            let _ = this.update(cx, |view, cx| {
                view.apply_models_result(models_result, settings_result, cx)
            });
            let result = bridge.list_threads().await;
            let _ = this.update(cx, |view, cx| view.apply_threads_result(result, cx));
        })
        .detach();

        let bridge = self.bridge.clone();
        let cwd = workspace_path();
        cx.spawn(async move |this, cx| {
            let result = bridge.list_permission_profiles(cwd).await;
            let _ = this.update(cx, |view, cx| {
                view.apply_permission_profiles_result(result, cx)
            });
        })
        .detach();
    }
}
