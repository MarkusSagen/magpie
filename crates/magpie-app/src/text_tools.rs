//! UI controller shared by external selections and the clipboard Transform entry.
use crate::{LauncherWindow, SelectionPrefs, TextActionRow, TextToolsWindow};
use magpie_app::{
    app_state::AppState,
    config::{Config, SelectionSettings},
};
use magpie_core::text_actions::{Action, Step, CATALOG};
use magpie_platform::selection::{self, BarStyle, Controller, Target};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
struct Pipeline {
    steps: Vec<Step>,
    selected: Option<usize>,
    recipe: Option<i64>,
}

struct State {
    input: String,
    target: Option<Target>,
    steps: Vec<Step>,
    selected: Option<usize>,
    recipe: Option<i64>,
    undo: Vec<Pipeline>,
    redo: Vec<Pipeline>,
    result: Option<String>,
    diagnostics: Option<(bool, bool, bool)>,
    native: Option<Controller>,
    settings: SelectionSettings,
}
impl State {
    fn pipeline(&self) -> Pipeline {
        Pipeline {
            steps: self.steps.clone(),
            selected: self.selected,
            recipe: self.recipe,
        }
    }

    fn record_edit(&mut self, previous: Pipeline) {
        if self.pipeline() != previous {
            if self.undo.len() == 64 {
                self.undo.remove(0);
            }
            self.undo.push(previous);
            self.redo.clear();
        }
    }

    fn restore(&mut self, redo: bool) -> bool {
        let current = self.pipeline();
        let previous = if redo {
            self.redo.pop().inspect(|_| self.undo.push(current))
        } else {
            self.undo.pop().inspect(|_| self.redo.push(current))
        };
        if let Some(pipeline) = previous {
            self.steps = pipeline.steps;
            self.selected = pipeline.selected;
            self.recipe = pipeline.recipe;
            true
        } else {
            false
        }
    }
}

pub struct TextTools {
    window: TextToolsWindow,
    state: Rc<RefCell<State>>,
    app: Arc<AppState>,
    generation: Arc<AtomicU64>,
    preview: Rc<slint::Timer>,
}
fn strings(v: Vec<String>) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        v.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}
fn preview_lines(text: &str) -> Vec<String> {
    let mut rows = text
        .lines()
        .take(500)
        .map(|l| l.chars().take(1000).collect())
        .collect::<Vec<String>>();
    if text.lines().count() > 500 || text.lines().any(|l| l.chars().count() > 1000) {
        rows.push("… preview truncated; the result keeps the complete text".into());
    }
    rows
}
impl TextTools {
    pub fn new(
        ui: &LauncherWindow,
        app: Arc<AppState>,
        cfg: &Config,
    ) -> Result<Rc<Self>, slint::PlatformError> {
        let tools = Rc::new(Self {
            window: TextToolsWindow::new()?,
            state: Rc::new(RefCell::new(State {
                input: String::new(),
                target: None,
                steps: Vec::new(),
                selected: None,
                recipe: None,
                undo: Vec::new(),
                redo: Vec::new(),
                result: None,
                diagnostics: None,
                native: None,
                settings: cfg.selection.clone(),
            })),
            app,
            generation: Arc::new(AtomicU64::new(0)),
            preview: Rc::new(slint::Timer::default()),
        });
        tools.window.set_theme_index(ui.get_theme_index());
        tools.window.set_primary_key(
            if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            }
            .into(),
        );
        let mut categories = CATALOG
            .iter()
            .map(|action| action.group)
            .collect::<Vec<_>>();
        categories.extend(["Saved cleaner", "Output actions"]);
        categories.sort_unstable();
        categories.dedup();
        tools.window.set_categories(strings(
            std::iter::once("All commands")
                .chain(categories)
                .map(str::to_owned)
                .collect(),
        ));
        tools.wire(ui);
        tools.catalog();
        tools.settings_props(ui);
        let timer = slint::Timer::default();
        let t = tools.clone();
        let w = ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(150),
            move || {
                if let Some(ui) = w.upgrade() {
                    t.tick(&ui);
                }
            },
        );
        // The timer owns the controller; the controller's lifetime is the event loop.
        // Retain the timer alongside it rather than leaking a native monitor.
        TIMERS.with(|timers| timers.borrow_mut().push(timer));
        Ok(tools)
    }
    pub fn open(&self, input: String, target: Option<Target>, initial: Option<Action>) {
        if std::env::var_os("MAGPIE_SELECTION_DEBUG").is_some() {
            eprintln!("selection: palette open");
        }
        {
            let mut s = self.state.borrow_mut();
            if let Some(n) = &mut s.native {
                n.dismiss();
            }
            s.input = input;
            s.target = target;
            s.steps = initial.map(Step::new).into_iter().collect();
            s.selected = (!s.steps.is_empty()).then_some(0);
            s.recipe = None;
            s.undo.clear();
            s.redo.clear();
            s.result = None;
        }
        self.window.set_saving_open(false);
        self.window.set_search("".into());
        self.window.set_name("".into());
        self.window.set_saved(false);
        self.window.set_error("".into());
        // Native selection actions may open this as the first visible Slint window.
        // Give winit an explicit client size before AppKit orders it front.
        self.window
            .window()
            .set_size(slint::LogicalSize::new(960., 680.));
        let _ = self.window.show();
        magpie_platform::os::window::raise_window_titled("Magpie — Text actions");
        self.window.invoke_prepare();
        self.catalog();
        self.update();
    }
    fn catalog(&self) {
        let q = self.window.get_search().to_lowercase();
        let category = self.window.get_category();
        let matches = |group: &str, text: &str| {
            (category == "All commands" || category == group)
                && q.split_whitespace().all(|term| text.contains(term))
        };
        let mut rows = CATALOG
            .iter()
            .filter(|d| {
                matches(
                    d.group,
                    &format!("{} {} {}", d.label, d.group, d.id).to_lowercase(),
                )
            })
            .map(|d| TextActionRow {
                id: d.id.into(),
                label: d.label.into(),
                group: d.group.into(),
            })
            .collect::<Vec<_>>();
        for (id, label) in [
            ("copy_original", "Copy original text"),
            ("save_result", "Save result as pinned clipboard entry"),
        ] {
            if matches("Output actions", &label.to_lowercase()) {
                rows.push(TextActionRow {
                    id: id.into(),
                    label: label.into(),
                    group: "Output actions".into(),
                });
            }
        }
        match self
            .app
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .text_recipes()
        {
            Ok(recipes) => {
                for r in recipes {
                    if matches("Saved cleaner", &r.name.to_lowercase()) {
                        rows.push(TextActionRow {
                            id: format!("recipe:{}", r.id).into(),
                            label: r.name.into(),
                            group: "Saved cleaner".into(),
                        });
                    }
                }
            }
            Err(e) => self.window.set_error(e.into()),
        }
        self.window.set_actions(ModelRc::new(VecModel::from(rows)));
    }
    fn update(&self) {
        let s = self.state.borrow();
        self.window.set_steps(strings(
            s.steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    format!(
                        "{}. {}{}",
                        i + 1,
                        if s.enabled { "" } else { "[off] " },
                        s.action.definition().label
                    )
                })
                .collect(),
        ));
        self.window
            .set_selected_step(s.selected.map(|i| i as i32).unwrap_or(-1));
        self.window
            .set_can_replace(s.target.as_ref().is_some_and(|t| t.editable));
        if let Some(step) = s.selected.and_then(|i| s.steps.get(i)) {
            let def = step.action.definition();
            self.window.set_first_label(
                match step.action {
                    Action::Split => "Separator",
                    Action::Join => "Join with",
                    Action::Replace => "Find",
                    _ => def.first,
                }
                .into(),
            );
            self.window.set_second_label(
                if step.action == Action::Replace {
                    "Replace with"
                } else {
                    def.second
                }
                .into(),
            );
            self.window.set_can_case(matches!(
                step.action,
                Action::SortAscending | Action::SortDescending | Action::Unique | Action::Replace
            ));
            self.window.set_can_regex(step.action == Action::Replace);
            self.window.set_first(step.text.clone().into());
            self.window.set_second(step.replacement.clone().into());
            self.window.set_sensitive(step.case_sensitive);
            self.window.set_regex(step.regex);
        } else {
            self.window.set_first_label("".into());
            self.window.set_second_label("".into());
            self.window.set_can_case(false);
            self.window.set_can_regex(false);
        }
        self.window.set_original(strings(preview_lines(&s.input)));
        self.window.set_saved(s.recipe.is_some());
        self.window.set_step_enabled(
            s.selected
                .and_then(|i| s.steps.get(i))
                .is_some_and(|step| step.enabled),
        );
        let stage = self
            .window
            .get_stage_preview()
            .then_some(s.selected)
            .flatten();
        let steps = stage.map_or(s.steps.as_slice(), |i| &s.steps[..=i]);
        let (input, steps) = (s.input.clone(), steps.to_vec());
        let show_stage = stage.is_some();
        drop(s);
        self.window.set_ready(false);
        self.window.set_error("".into());
        self.state.borrow_mut().result = None;
        let revision = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.preview.stop();
        if steps.is_empty() {
            self.window.set_result(strings(Vec::new()));
            self.window.set_error("".into());
            return;
        }
        let preview = Rc::downgrade(&self.preview);
        let generation = self.generation.clone();
        let weak = self.window.as_weak();
        let state = self.state.clone();
        slint::Timer::single_shot(Duration::from_millis(120), move || {
            if generation.load(Ordering::Relaxed) != revision {
                return;
            }
            let (send, recv) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let evaluate = magpie_core::text_actions::evaluate;
                let preview = match steps.split_last() {
                    Some((last, earlier)) if show_stage => match evaluate(&input, earlier) {
                        Ok(before) => {
                            let after = evaluate(&before, std::slice::from_ref(last));
                            (Some(before), after)
                        }
                        Err(error) => (None, Err(error)),
                    },
                    _ => (None, evaluate(&input, &steps)),
                };
                let _ = send.send(preview);
            });
            let Some(poll) = preview.upgrade() else {
                return;
            };
            // A timer's stopped callback remains installed. Capture a Weak timer
            // so each edit cannot create a self-retaining timer/callback cycle.
            let timer = Rc::downgrade(&poll);
            poll.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(30),
                move || {
                    if generation.load(Ordering::Relaxed) != revision {
                        if let Some(timer) = timer.upgrade() {
                            timer.stop();
                        }
                        return;
                    }
                    if let Ok((before, result)) = recv.try_recv() {
                        if let Some(timer) = timer.upgrade() {
                            timer.stop();
                        }
                        if let Some(ui) = weak.upgrade() {
                            if let Some(before) = before {
                                ui.set_original(strings(preview_lines(&before)));
                            }
                            match result {
                                Ok(text) => {
                                    ui.set_result(strings(preview_lines(&text)));
                                    ui.set_error("".into());
                                    ui.set_ready(true);
                                    state.borrow_mut().result = Some(text);
                                }
                                Err(e) => {
                                    ui.set_result(strings(Vec::new()));
                                    ui.set_error(e.into());
                                    ui.set_ready(false);
                                }
                            }
                        }
                    }
                },
            );
        });
    }
    fn dismiss(&self) {
        if std::env::var_os("MAGPIE_SELECTION_DEBUG").is_some() {
            eprintln!("selection: palette dismiss");
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.preview.stop();
        {
            let mut s = self.state.borrow_mut();
            s.target = None;
            s.input.clear();
            s.result = None;
            if let Some(n) = &mut s.native {
                n.dismiss();
            }
        }
        self.window.set_original(strings(Vec::new()));
        self.window.set_result(strings(Vec::new()));
        self.window.set_ready(false);
        let _ = self.window.hide();
    }
    fn settings_props(&self, ui: &LauncherWindow) {
        let s = self.state.borrow();
        let p = ui.global::<SelectionPrefs>();
        p.set_enabled(s.settings.enabled);
        p.set_apps(s.settings.apps.join(", ").into());
        p.set_actions(s.settings.actions.join(", ").into());
        p.set_shortcuts(s.settings.shortcuts.join(", ").into());
        p.set_status(if !selection::supported(){"Selection bar: not implemented on this OS. Text actions and cleaners are available."}else if !magpie_platform::accessibility_trusted(){"Accessibility permission is needed. Writing-app MVP: TextEdit and Notes."}else{"Writing-app MVP: TextEdit and Notes. Browser, developer and terminal support is unverified."}.into());
    }
    fn tick(self: &Rc<Self>, ui: &LauncherWindow) {
        self.window.set_theme_index(ui.get_theme_index());
        let mut s = self.state.borrow_mut();
        let settings = s.settings.clone();
        let trusted = magpie_platform::accessibility_trusted();
        let native_enabled = settings.enabled && selection::supported() && trusted;
        if native_enabled && s.native.is_none() {
            s.native = Controller::new();
        }
        if !native_enabled {
            s.native = None;
        }
        let diagnostics = (settings.enabled, trusted, s.native.is_some());
        if s.diagnostics != Some(diagnostics) {
            if std::env::var_os("MAGPIE_SELECTION_DEBUG").is_some() {
                eprintln!(
                    "selection: enabled={} trusted={} native={} window-visible={}",
                    diagnostics.0,
                    diagnostics.1,
                    diagnostics.2,
                    self.window.window().is_visible()
                );
            }
            s.diagnostics = Some(diagnostics);
        }

        ui.global::<SelectionPrefs>().set_status(
            if !selection::supported() {
                "Selection bar: not implemented on this OS. Text tools are available."
            } else if !settings.enabled {
                "Automatic selection bar is off."
            } else if !trusted {
                "Accessibility permission is needed. MVP targets: TextEdit and Notes."
            } else {
                "MVP targets: TextEdit and Notes. Other apps are unverified."
            }
            .into(),
        );

        if let Some(native) = &mut s.native {
            native.restore_clipboard();
        }
        if self.window.window().is_visible() || !settings.enabled {
            let _ = selection::take_action();
            if let Some(n) = &mut s.native {
                n.dismiss();
            }
            if !settings.enabled {
                s.target = None;
            }
            return;
        }
        if let Some(slot) = selection::take_action() {
            let id = settings
                .actions
                .get(slot)
                .cloned()
                .unwrap_or_else(|| "more".into());
            let target = s
                .target
                .clone()
                .filter(|target| s.native.as_ref().is_some_and(|n| n.valid(target)));
            drop(s);
            if let Some(target) = target {
                if id == "copy" {
                    match magpie_platform::platform_clipboard()
                        .and_then(|mut c| c.set_text(&target.text))
                    {
                        Ok(()) => {
                            if let Some(n) = &mut self.state.borrow_mut().native {
                                n.finish(&target);
                            }
                        }
                        Err(e) => {
                            self.open(target.text.clone(), Some(target), None);
                            self.window.set_error(e.into());
                        }
                    }
                } else if let Some(action) = Action::from_id(&id) {
                    if matches!(
                        action,
                        Action::SortAscending
                            | Action::SortDescending
                            | Action::Unique
                            | Action::Trim
                            | Action::Upper
                            | Action::Lower
                    ) && target.editable
                    {
                        match magpie_core::text_actions::evaluate(
                            &target.text,
                            &[Step::new(action)],
                        ) {
                            Ok(result) => {
                                let error = self
                                    .state
                                    .borrow_mut()
                                    .native
                                    .as_mut()
                                    .and_then(|n| n.replace(&target, &result).err());
                                if let Some(error) = error {
                                    self.open(
                                        target.text.clone(),
                                        Some(target.clone()),
                                        Some(action),
                                    );
                                    self.window.set_error(error.into());
                                }
                            }
                            Err(e) => {
                                self.open(target.text.clone(), Some(target.clone()), Some(action));
                                self.window.set_error(e.into());
                            }
                        }
                    } else {
                        self.open(target.text.clone(), Some(target), Some(action));
                    }
                } else {
                    self.open(target.text.clone(), Some(target), None);
                }
            }
            return;
        }
        let target = s.native.as_mut().and_then(|n| n.poll(&settings.apps));
        if let Some(target) = target {
            let changed = s.target.as_ref().is_none_or(|old| !old.same_as(&target));
            let labels = settings
                .actions
                .iter()
                .map(|id| {
                    match id.as_str() {
                        "copy" => "Copy",
                        "sort_ascending" => "Sort ↑",
                        "sort_descending" => "Sort ↓",
                        "split" => "Split",
                        "join" => "Join",
                        "transform" => "Transform",
                        "more" => "More",
                        id => Action::from_id(id)
                            .map(|a| a.definition().label)
                            .unwrap_or("More"),
                    }
                    .to_string()
                })
                .collect::<Vec<_>>();
            let style = ui.global::<crate::TextBarTheme>();
            let to_rgba = |c: slint::Color| {
                [
                    c.red() as f64 / 255.,
                    c.green() as f64 / 255.,
                    c.blue() as f64 / 255.,
                    c.alpha() as f64 / 255.,
                ]
            };
            if let Some(n) = &mut s.native {
                if changed {
                    n.dismiss();
                }
                n.show(
                    &target,
                    &labels,
                    &settings.shortcuts,
                    &BarStyle {
                        background: to_rgba(style.get_background()),
                        foreground: to_rgba(style.get_foreground()),
                    },
                );
            }
            s.target = Some(target);
        } else {
            if let Some(n) = &mut s.native {
                n.dismiss();
            }
            s.target = None;
        }
    }
    fn show_example(&self, id: &str) {
        let default_step = Action::from_id(id).map(Step::new);
        let state = self.state.borrow();
        let Some(mut step) =
            default_step.or_else(|| state.selected.map(|index| state.steps[index].clone()))
        else {
            return;
        };
        drop(state);
        use Action::*;
        let sample = match step.action {
            SortAscending | SortDescending | Unique | Reverse | Join | Number | Sql | Count => {
                "pear\napple\npear"
            }
            Trim | Whitespace | RemoveEmpty => "  pear   and   apple  \n\nbanana",
            Split => "pear, apple, pear",
            Prefix | RemovePrefix => "- pear\n- apple",
            Suffix | RemoveSuffix => "pear.\napple.",
            Replace => "pear and apple\npear",
            Camel | Pascal | Snake | Kebab | Screaming | Train | Title | Upper | Lower => {
                "Magpie text actions"
            }
            Base64Decode => "TWFncGll",
            UrlDecode => "pear%20%26%20apple",
            HtmlDecode => "pear &amp; apple",
            StripTags => "<p>Pear &amp; apple</p><p>Banana</p>",
            Base64Encode | UrlEncode | HtmlEncode | Sha256 | Md5 => "pear & apple",
        };
        if step.action == Replace && step.text.is_empty() {
            step.text = "pear".into();
            step.replacement = "plum".into();
        }
        step.enabled = true;
        self.window
            .set_example_title(step.action.definition().label.into());
        self.window
            .set_example_before(strings(preview_lines(sample)));
        let preview =
            magpie_core::text_actions::evaluate(sample, &[step]).unwrap_or_else(|error| error);
        self.window
            .set_example_after(strings(preview_lines(&preview)));
        self.window.set_example_open(true);
    }

    fn choose(&self, id: &str, replace: bool) {
        if replace && Action::from_id(id).is_none() {
            self.window
                .set_error("Choose a transform command to replace this step".into());
            return;
        }
        match id {
            "copy_original" => {
                if let Err(error) = magpie_platform::platform_clipboard()
                    .and_then(|mut clipboard| clipboard.set_text(&self.state.borrow().input))
                {
                    self.window.set_error(error.into());
                }
                return;
            }
            "save_result" => {
                let result = self.state.borrow().result.clone();
                let message = if let Some(text) = result {
                    match self
                        .app
                        .store
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .create_snippet(&text, crate::runtime::now_ms())
                    {
                        Ok(_) => "Saved as a pinned clipboard entry".into(),
                        Err(error) => error.to_string(),
                    }
                } else {
                    "Choose valid actions and wait for the preview before saving the result".into()
                };
                self.window.set_error(message.into());
                return;
            }
            _ => {}
        }
        let mut state = self.state.borrow_mut();
        let previous = state.pipeline();
        if let Some(action) = Action::from_id(id) {
            let mut step = Step::new(action);
            if replace {
                let Some(index) = state.selected else {
                    return;
                };
                step.enabled = state.steps[index].enabled;
                state.steps[index] = step;
            } else if state.steps.len() < 64 {
                state.steps.push(step);
                state.selected = Some(state.steps.len() - 1);
            } else {
                self.window
                    .set_error("A cleaner can contain at most 64 steps".into());
                return;
            }
        } else if let Some(id) = id
            .strip_prefix("recipe:")
            .and_then(|id| id.parse::<i64>().ok())
        {
            let recipes = self
                .app
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .text_recipes();
            match recipes {
                Ok(recipes) => {
                    let Some(recipe) = recipes.into_iter().find(|recipe| recipe.id == id) else {
                        self.window
                            .set_error("This cleaner is no longer available".into());
                        return;
                    };
                    state.steps = recipe.steps;
                    state.recipe = Some(id);
                    state.selected = (!state.steps.is_empty()).then_some(0);
                    self.window.set_name(recipe.name.into());
                }
                Err(error) => {
                    self.window.set_error(error.into());
                    return;
                }
            }
        }
        state.record_edit(previous);
        drop(state);
        self.update();
    }

    fn restore_pipeline(&self, redo: bool) {
        let mut state = self.state.borrow_mut();
        if !state.restore(redo) {
            return;
        }
        let name = state.recipe.and_then(|id| {
            self.app
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .text_recipes()
                .ok()?
                .into_iter()
                .find(|recipe| recipe.id == id)
                .map(|recipe| recipe.name)
        });
        if name.is_none() {
            state.recipe = None;
        }
        self.window.set_name(name.unwrap_or_default().into());
        drop(state);
        self.update();
    }

    fn wire(self: &Rc<Self>, ui: &LauncherWindow) {
        macro_rules! on {
            ($method:ident,$body:expr) => {{
                let weak = Rc::downgrade(self);
                self.window.$method(move || {
                    if let Some(t) = weak.upgrade() {
                        $body(t);
                    }
                });
            }};
        }
        on!(on_filter, |t: Rc<Self>| t.catalog());
        on!(on_undo, |t: Rc<Self>| t.restore_pipeline(false));
        on!(on_redo, |t: Rc<Self>| t.restore_pipeline(true));
        on!(on_preview_stage, |t: Rc<Self>| t.update());
        {
            let weak = Rc::downgrade(self);
            self.window.on_example(move |id| {
                if let Some(tools) = weak.upgrade() {
                    tools.show_example(id.as_str());
                }
            });
        }
        on!(on_duplicate_step, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            if let Some(index) = s.selected.filter(|_| s.steps.len() < 64) {
                let previous = s.pipeline();
                let step = s.steps[index].clone();
                s.steps.insert(index + 1, step);
                s.selected = Some(index + 1);
                s.record_edit(previous);
            }
            drop(s);
            t.update();
        });
        {
            let weak = Rc::downgrade(self);
            self.window.on_choose(move |id, replace| {
                if let Some(t) = weak.upgrade() {
                    t.choose(id.as_str(), replace);
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            self.window.on_select_step(move |i| {
                if let Some(t) = weak.upgrade() {
                    if i >= 0 && (i as usize) < t.state.borrow().steps.len() {
                        t.state.borrow_mut().selected = Some(i as usize);
                        t.update();
                    }
                }
            });
        }
        on!(on_configure, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            let previous = s.pipeline();
            if let Some(i) = s.selected {
                if let Some(step) = s.steps.get_mut(i) {
                    step.text = t.window.get_first().to_string();
                    step.replacement = t.window.get_second().to_string();
                    step.case_sensitive = t.window.get_sensitive();
                    step.regex = t.window.get_regex();
                }
            }
            s.record_edit(previous);
            drop(s);
            t.update();
        });
        on!(on_reset, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            let previous = s.pipeline();
            s.steps.clear();
            s.selected = None;
            s.recipe = None;
            s.record_edit(previous);
            drop(s);
            t.window.set_name("".into());
            t.update();
        });
        on!(on_remove_step, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            let previous = s.pipeline();
            if let Some(i) = s.selected {
                s.steps.remove(i);
                s.selected =
                    (!s.steps.is_empty()).then_some(i.min(s.steps.len().saturating_sub(1)));
            }
            s.record_edit(previous);
            drop(s);
            t.update();
        });
        on!(on_toggle_step, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            let previous = s.pipeline();
            if let Some(i) = s.selected {
                s.steps[i].enabled = !s.steps[i].enabled;
            }
            s.record_edit(previous);
            drop(s);
            t.update();
        });
        {
            let weak = Rc::downgrade(self);
            self.window.on_move_step(move |delta| {
                if let Some(t) = weak.upgrade() {
                    let mut s = t.state.borrow_mut();
                    let previous = s.pipeline();
                    if let Some(i) = s.selected {
                        let j = i as i32 + delta;
                        if j >= 0 && (j as usize) < s.steps.len() {
                            s.steps.swap(i, j as usize);
                            s.selected = Some(j as usize);
                        }
                    }
                    s.record_edit(previous);
                    drop(s);
                    t.update();
                }
            });
        }
        on!(on_save, |t: Rc<Self>| {
            let mut s = t.state.borrow_mut();
            match t
                .app
                .store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .save_text_recipe(
                    s.recipe,
                    &t.window.get_name(),
                    &s.steps,
                    crate::runtime::now_ms(),
                ) {
                Ok(id) => {
                    s.recipe = Some(id);
                    t.window.set_saved(true);
                    t.window.set_error("".into());
                    t.window.set_saving_open(false);
                }
                Err(e) => t.window.set_error(e.into()),
            }
            drop(s);
            t.catalog();
        });
        on!(on_delete_recipe, |t: Rc<Self>| {
            let id = t.state.borrow().recipe;
            if let Some(id) = id {
                match t
                    .app
                    .store
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .delete_text_recipe(id)
                {
                    Ok(()) => {
                        t.state.borrow_mut().recipe = None;
                        t.window.set_name("".into());
                        t.window.set_saved(false);
                    }
                    Err(e) => t.window.set_error(e.into()),
                }
                t.catalog();
            }
        });
        on!(on_copy_result, |t: Rc<Self>| {
            if let Some(result) = t.state.borrow().result.clone() {
                if let Err(e) =
                    magpie_platform::platform_clipboard().and_then(|mut c| c.set_text(&result))
                {
                    t.window.set_error(e.into());
                }
            }
        });
        {
            let weak = Rc::downgrade(self);
            self.window.on_dismiss(move || {
                if let Some(t) = weak.upgrade() {
                    slint::Timer::single_shot(Duration::ZERO, move || t.dismiss());
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            self.window.window().on_close_requested(move || {
                if let Some(t) = weak.upgrade() {
                    slint::Timer::single_shot(Duration::ZERO, move || t.dismiss());
                }
                slint::CloseRequestResponse::KeepWindowShown
            });
        }
        {
            let weak = Rc::downgrade(self);
            self.window.on_replace_result(move || {
                if let Some(t) = weak.upgrade() {
                    let revision = t.generation.load(Ordering::Relaxed);
                    let (target, result) = {
                        let state = t.state.borrow();
                        (state.target.clone(), state.result.clone())
                    };
                    let (Some(target), Some(result)) = (target, result) else {
                        return;
                    };
                    if t.window.get_stage_preview() {
                        return;
                    }
                    // Focus changes must happen outside Slint's synchronous keyDown.
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        if t.generation.load(Ordering::Relaxed) != revision {
                            return;
                        }
                        let unavailable =
                            || "Selection replacement is no longer available".to_string();
                        let focus = t
                            .state
                            .borrow()
                            .native
                            .as_ref()
                            .ok_or_else(unavailable)
                            .and_then(|native| native.focus(&target));
                        if let Err(error) = focus {
                            t.window.set_error(error.into());
                            return;
                        }
                        slint::Timer::single_shot(Duration::from_millis(100), move || {
                            if t.generation.load(Ordering::Relaxed) != revision {
                                return;
                            }
                            let replacement = t
                                .state
                                .borrow_mut()
                                .native
                                .as_mut()
                                .ok_or_else(unavailable)
                                .and_then(|native| native.replace(&target, &result));
                            match replacement {
                                Ok(()) => t.dismiss(),
                                Err(error) => t.window.set_error(error.into()),
                            }
                        });
                    });
                }
            });
        }
        let prefs = ui.global::<SelectionPrefs>();
        {
            let weak = Rc::downgrade(self);
            let w = ui.as_weak();
            prefs.on_save(move || {
                if let (Some(t), Some(ui)) = (weak.upgrade(), w.upgrade()) {
                    let p = ui.global::<SelectionPrefs>();
                    let split = |s: slint::SharedString| {
                        s.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    };
                    let settings = SelectionSettings {
                        enabled: p.get_enabled(),
                        apps: split(p.get_apps()),
                        actions: split(p.get_actions()),
                        shortcuts: split(p.get_shortcuts()),
                    };
                    if let Err(error) = settings.validate() {
                        p.set_status(error.into());
                        return;
                    }
                    let path = crate::runtime::data_dir().join("config.toml");
                    let mut cfg = magpie_app::config::load_or_default(&path);
                    cfg.selection = settings.clone();
                    if let Err(e) = magpie_app::config::save(&cfg, &path) {
                        p.set_status(e.to_string().into());
                        return;
                    }
                    let mut s = t.state.borrow_mut();
                    s.settings = settings;
                    if let Some(n) = &mut s.native {
                        n.dismiss();
                    }
                    drop(s);
                    t.settings_props(&ui);
                }
            });
        }
        prefs.on_permission(|| {
            magpie_platform::prompt_accessibility();
            magpie_platform::open_accessibility_settings();
        });
    }
}
thread_local! {static TIMERS:RefCell<Vec<slint::Timer>>=const{RefCell::new(Vec::new())};}
