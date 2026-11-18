//! 组件生命周期的共同接线；业务语义仍由各服务拥有，保持原有锁范围和调用顺序。
use crate::{
    store::{err, AppResult},
    widgets, Core,
};
use tauri::{AppHandle, Manager};

pub(crate) fn close_session(core: &Core, window: &str, token: &str) -> AppResult<()> {
    let mut sessions = core.sessions.0.lock().map_err(err)?;
    if sessions.get(token).is_some_and(|s| s.window == window) {
        core.pomodoro_windows
            .service
            .session_closed(window, token)?;
        sessions.remove(token);
        core.ai.cancel_session(&core.store, token)?;
        core.coin_live
            .release_owners(&std::collections::HashSet::from([token.to_owned()]))?;
    }
    Ok(())
}

// 调用方已经持有native_actions；不在这里重复加锁。
pub(crate) fn before_package_switch(core: &Core, staged: &widgets::StagedPackage) -> AppResult<()> {
    // 升级提交前撤销助手网页授权；失败时保留旧包和登录资料，可由控制卡重新打开。
    if staged.manifest.id == crate::pomodoro_window::PACKAGE {
        for (id, _, _) in &staged.instances {
            core.pomodoro_windows.service.revoke(id)?;
        }
    }
    if staged.manifest.id == crate::job_protocol::PACKAGE {
        for (id, _, _) in &staged.instances {
            core.jobs.revoke(id)?;
        }
    }
    if staged.manifest.id == crate::assistant_host::PACKAGE {
        for (id, _, _) in &staged.instances {
            core.assistant.cancel(id)?;
        }
    }
    Ok(())
}

pub(crate) fn after_package_switch(app: &AppHandle, staged: &widgets::StagedPackage) {
    let core = app.state::<Core>();
    if staged.manifest.id == crate::assistant_host::PACKAGE {
        for (id, _, _) in &staged.instances {
            core.assistant.kick(app, id);
        }
    }
    if staged.manifest.id == crate::job_protocol::PACKAGE {
        for (id, _, _) in &staged.instances {
            core.jobs.kick(app, id);
        }
    }
    if staged.manifest.id == crate::pomodoro_window::PACKAGE {
        core.pomodoro_windows.tick(app);
    }
}

pub(crate) fn remove_instance(app: &AppHandle, id: &str) -> AppResult<()> {
    let core = app.state::<Core>();
    let removed = crate::widgets::remove_instance_before(&core.store, &core.sessions, id, || {
        core.pomodoro_windows.service.revoke(id)?;
        core.jobs.revoke(id)?;
        core.assistant.cancel(id)
    });
    let cleanup = core.assistant.cleanup(app);
    let job_cleanup = core.jobs.cleanup_instance(app, id);
    let pomodoro_cleanup = core.pomodoro_windows.cleanup(app);
    removed?;
    cleanup?;
    job_cleanup?;
    pomodoro_cleanup?;
    core.ai.forget(id)?;
    core.clipboard.forget(id)?;
    core.coin_index.forget(id)?;
    Ok(())
}

pub(crate) fn after_instance_hidden(app: &AppHandle, id: &str) -> AppResult<()> {
    app.state::<Core>().pomodoro_windows.service.revoke(id)?;
    app.state::<Core>().pomodoro_windows.kick(app);
    app.state::<Core>()
        .jobs
        .hide_instance(&app.state::<Core>().store, id)?;
    app.state::<Core>().jobs.kick(app, id);
    Ok(())
}

// 与实例删除共用同一SQLite事务；失败仍由调用方回滚。
pub(crate) fn remove_persisted(
    transaction: &rusqlite::Transaction<'_>,
    instance: &str,
) -> AppResult<()> {
    transaction
        .execute(
            "DELETE FROM app_settings WHERE key=?1",
            [format!("media_directory:{instance}")],
        )
        .map_err(err)?;
    transaction
        .execute(
            "DELETE FROM app_settings WHERE key=?1",
            [format!("file_launcher:{instance}")],
        )
        .map_err(err)?;
    crate::water::remove_data(&transaction, instance)?;
    let mut groups = crate::store::panel_groups(&transaction)?;
    if groups.membership.remove(instance).is_some() {
        crate::store::save_panel_groups(&transaction, &groups)?;
    }
    crate::weather::remove_data(&transaction, instance)?;
    crate::ai_host::remove_data(&transaction, instance)?;
    crate::clipboard_history::remove_data(&transaction, instance)?;
    crate::clipboard_actions::remove_data(&transaction, instance)?;
    crate::assistant_state::remove_data(&transaction, instance)?;
    crate::job_state::remove_data(&transaction, instance)?;
    crate::job_knowledge::remove_data(&transaction, instance)?;
    crate::shared_library::remove_data(&transaction, instance)?;
    crate::labor::remove_data(&transaction, instance)?;
    crate::reminder_store::remove_data(&transaction, instance)?;
    crate::todo::remove_data(&transaction, instance)?;
    crate::pomodoro::remove_data(&transaction, instance)?;
    crate::pomodoro_window::remove_data(&transaction, instance)?;
    Ok(())
}

pub(crate) fn before_exit(app: &AppHandle) {
    let _ = app.state::<Core>().pomodoro_windows.service.stop();
    if let Err(error) = app.state::<Core>().pomodoro_windows.cleanup(app) {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app
        .state::<Core>()
        .jobs
        .shutdown(&app.state::<Core>().store)
    {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app.state::<Core>().jobs.cleanup(app) {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app
        .state::<Core>()
        .assistant
        .stop()
        .and_then(|_| app.state::<Core>().assistant.cleanup(app))
    {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app.state::<Core>().assistant.stop_shortcuts(app) {
        app.state::<Core>().report(error);
    };
}

pub(crate) fn after_geometry_saved(app: &AppHandle) {
    if let Err(error) = app.state::<Core>().coin_live.stop_all() {
        app.state::<Core>().report(error);
    }
    app.state::<Core>()
        .quitting
        .store(true, std::sync::atomic::Ordering::Relaxed);
    app.state::<Core>().reminders.stop();
    app.state::<Core>().ai.stop();
    if let Err(error) = app
        .state::<Core>()
        .clipboard
        .stop(&app.state::<Core>().store)
    {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app.state::<Core>().clipboard.stop_shortcuts(app) {
        app.state::<Core>().report(error);
    }
    if let Err(error) = app.state::<Core>().keyboard.stop() {
        app.state::<Core>().report(error);
    }
}

pub(crate) fn tick(app: &AppHandle) {
    let core = app.state::<Core>();
    core.water.dispatch(app);
    core.labor.dispatch(app);
    core.todo.dispatch(app);
    core.pomodoro.dispatch(app);
    core.pomodoro_windows.tick(app);
    core.reminders.dispatch(app);
    core.assistant.tick(app);
    core.jobs.tick(app);
    if let Ok(sessions) = core.sessions.0.lock() {
        let owners = sessions.keys().cloned().collect();
        if let Err(error) = core.coin_live.retain_owners(&owners) {
            core.report(error);
        }
    };
}
