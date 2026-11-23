//! The update bubble and dialog (ui/update.slint): the daily check on a
//! background thread, and what the dialog's buttons do. Wired once at startup.

use super::{installed::installed, Event, Release};
use crate::{AppWindow, Updater};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The first check waits until Archtoys is up (never during the start).
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn install(ui: &AppWindow, persist: impl Fn() + Clone + 'static) {
    let found: Arc<Mutex<Option<Release>>> = Arc::new(Mutex::new(None));
    let checking = Arc::new(AtomicBool::new(ui.global::<Updater>().get_check()));
    let g = ui.global::<Updater>();
    g.set_current(env!("CARGO_PKG_VERSION").into());

    let (c, p) = (checking.clone(), persist.clone());
    let weak = ui.as_weak();
    g.on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            c.store(ui.global::<Updater>().get_check(), Ordering::Relaxed);
        }
        p();
    });

    let (weak, f, p) = (ui.as_weak(), found.clone(), persist.clone());
    g.on_skip(move || {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<Updater>();
        if let Some(r) = f.lock().unwrap().as_ref() {
            crate::config::set_update_skipped(&r.version);
        }
        g.set_dialog_open(false);
        g.set_state(0);
        p();
    });

    let f = found.clone();
    g.on_release_page(move || {
        let page = f.lock().unwrap().as_ref().map(|r| r.page.clone()).unwrap_or_default();
        crate::open_link(if page.is_empty() { "https://github.com/Mujtaba1i/Archtoys/releases/latest" } else { &page });
    });

    let p = persist.clone();
    g.on_restart(move || {
        p();
        let err = super::restart(installed());
        eprintln!("Couldn't restart Archtoys: {err}");
    });

    let (weak, f) = (ui.as_weak(), found.clone());
    g.on_update(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(release) = f.lock().unwrap().clone() else { return };
        let g = ui.global::<Updater>();
        let steps: Vec<SharedString> = super::steps(installed()).into_iter().map(Into::into).collect();
        g.set_step(steps.first().cloned().unwrap_or_default());
        g.set_steps(ModelRc::from(Rc::new(VecModel::from(steps))));
        g.set_step_index(0);
        g.set_progress(0.0);
        g.set_state(2);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let count = super::steps(installed()).len().max(1) as f32;
            let at = Arc::new(Mutex::new((0usize, 0f32)));
            let on_event = {
                let (weak, at) = (weak.clone(), at.clone());
                move |event: Event| {
                    let (step, inner) = {
                        let mut at = at.lock().unwrap();
                        match event {
                            Event::Step(i) => *at = (i, 0.0),
                            Event::Percent(p) => at.1 = p,
                        }
                        *at
                    };
                    let progress = ((step as f32 + inner) / count).min(0.99);
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<Updater>();
                        g.set_step_index(step as i32);
                        if let Some(name) = g.get_steps().row_data(step) {
                            g.set_step(name);
                        }
                        g.set_progress(progress.max(g.get_progress()));
                    });
                }
            };
            // a step with no percent of its own (building) still moves a little
            let ticking = Arc::new(AtomicBool::new(true));
            {
                let (weak, ticking, at) = (weak.clone(), ticking.clone(), at.clone());
                std::thread::spawn(move || {
                    while ticking.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(700));
                        let (step, _) = *at.lock().unwrap();
                        let ceiling = ((step as f32 + 0.9) / count).min(0.99);
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            let g = ui.global::<Updater>();
                            let p = g.get_progress();
                            if p < ceiling {
                                g.set_progress(p + (ceiling - p) * 0.04);
                            }
                        });
                    }
                });
            }
            let result = super::run(installed(), &release, &on_event);
            ticking.store(false, Ordering::Relaxed);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<Updater>();
                match result {
                    Ok(()) => {
                        g.set_progress(1.0);
                        g.set_state(3);
                    }
                    Err(e) => {
                        g.set_error(e.into());
                        g.set_state(4);
                    }
                }
                g.set_dialog_open(true);
            });
        });
    });

    // the check: after the start, then once a day while Archtoys runs
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if checking.load(Ordering::Relaxed) {
                if let Some(release) = super::available(&crate::config::update_skipped()) {
                    *found.lock().unwrap() = Some(release.clone());
                    let _ = weak.upgrade_in_event_loop(move |ui| show(&ui, &release));
                }
            }
            std::thread::sleep(EVERY);
        }
    });
}

fn show(ui: &AppWindow, release: &Release) {
    let g = ui.global::<Updater>();
    // never interrupt an update that is running or done
    if g.get_state() >= 2 && g.get_state() != 4 {
        return;
    }
    let notes: Vec<SharedString> = release.notes.iter().map(|n| n.as_str().into()).collect();
    g.set_version(release.version.as_str().into());
    g.set_notes(ModelRc::from(Rc::new(VecModel::from(notes))));
    g.set_can_install(super::can_install(installed()));
    g.set_state(1);
}
