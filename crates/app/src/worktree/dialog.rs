//! The "Create worktree" view (Orca's dialog): Project / Run on selects, "Create from" tabs with a
//! search field (or the prefilled task as a chip), Agent select, Advanced options, Create more.

use agentdeck_core::agents::AgentKind;
use agentdeck_core::git::branch_worktree::{self, Start};
use agentdeck_core::hosts::Host;
use agentdeck_core::logic::deck::{PaneSpec, new_id, new_tab};
use agentdeck_core::logic::new_worktree::{
    GitHubKind, Smart, TaskSource, WorktreePrefill, branch_from_name, classify, filter_branches, parse_github,
    parse_linear, valid_branch,
};
use agentdeck_core::logic::text::{AGENT_KINDS, agent_name, split_args};
use agentdeck_core::logic::theme::{FONT_MONO, Palette, RADIUS_MD};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::panes::agent_logo;
use crate::state::Deck;
use crate::theme::{hsla, hsla_a, palette, ui};
use agentdeck_core::logic::theme::{RADIUS_LG, RADIUS_SM};

/// Project dots: a stable color per workspace.
const DOT_COLORS: [u32; 6] = [0xef4444, 0xf59e0b, 0x22c55e, 0x3b82f6, 0xa855f7, 0xec4899];
/// Branch rows listed under the field.
const BRANCH_ROWS: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Smart,
    GitHub,
    Linear,
    Branch,
    Name,
}

const MODES: [(Mode, &str, IconName); 5] = [
    (Mode::Smart, "Smart", IconName::Sparkles),
    (Mode::GitHub, "GitHub", IconName::Github),
    (Mode::Linear, "Linear", IconName::SquareKanban),
    (Mode::Branch, "Branch", IconName::GitBranch),
    (Mode::Name, "Name", IconName::Type),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Project,
    Host,
    Agent,
}

/// What to create, decided on the UI thread from the mode, the text and the task chip.
enum Plan {
    Start(Start),
    /// Fetched with gh first: a pull request keeps its branch, an issue gets a new one.
    GitHub(Option<GitHubKind>, u64),
    /// A new branch named from the task.
    Task(TaskSource),
}

enum Target {
    Local { cwd: String, parent: String, setup: String },
    Remote { host: Host, dir: String },
}

/// Background job: everything it needs, nothing from the UI.
struct Job {
    plan: Plan,
    target: Target,
    short: String,
    base: String,
}

struct Created {
    path: String,
    branch: String,
    task: Option<TaskSource>,
}

/// Blocking: gh (GitHub tasks), git worktree add (local or over ssh), setup.
fn run_job(job: Job) -> Result<Created, String> {
    let (start, task) = match job.plan {
        Plan::Start(start) => (start, None),
        Plan::Task(task) => (Start::New { branch: task.branch(), base: job.base.clone() }, Some(task)),
        Plan::GitHub(kind, number) => {
            let Target::Local { cwd, .. } = &job.target else {
                return Err("GitHub needs the project on this computer: pick \"This computer\".".into());
            };
            let (task, head) = branch_worktree::gh_task(cwd, kind, number)?;
            let start = match (&task, head) {
                (TaskSource::GitHub { kind: GitHubKind::PullRequest, number, .. }, Some(head)) => {
                    Start::Existing(branch_worktree::pr_branch(cwd, *number, &head)?)
                }
                _ => Start::New { branch: task.branch(), base: job.base.clone() },
            };
            (start, Some(task))
        }
    };
    let path = match &job.target {
        Target::Local { cwd, parent, setup } => branch_worktree::create(cwd, parent, &job.short, &start, setup)?,
        Target::Remote { host, dir } => {
            let base = match &start {
                Start::New { base, .. } => base.as_str(),
                Start::Existing(_) => "",
            };
            host.create_worktree(dir, &job.short, start.branch(), base)?
        }
    };
    Ok(Created { path, branch: start.branch().to_owned(), task })
}

/// Starts the agent in the new session; with a task, its prompt goes once the agent is ready.
fn start_agent(deck: Entity<Deck>, spec: PaneSpec, tab_id: String, prompt: Option<String>, cx: &mut App) {
    let Some(prompt) = prompt else {
        crate::panes::runtime::create_pane(spec, &tab_id, false, cx);
        return;
    };
    cx.spawn(async move |cx| {
        if !crate::palette::race_one(deck.clone(), spec, tab_id, prompt, cx).await {
            cx.update(|cx| deck.update(cx, |d, cx| d.toast("The agent did not start: the task was not sent.", cx)));
        }
    })
    .detach();
}

pub struct CreateWorktree {
    deck: Entity<Deck>,
    /// Workspace id.
    project: String,
    /// None = this computer, else a saved host id.
    host: Option<String>,
    mode: Mode,
    query: Entity<InputState>,
    /// Task chip (prefill); replaces the field until cleared.
    task: Option<TaskSource>,
    agent: Option<AgentKind>,
    args: Entity<InputState>,
    show_args: bool,
    advanced: bool,
    base: Entity<InputState>,
    location: Entity<InputState>,
    chat: bool,
    create_more: bool,
    menu: Option<Menu>,
    /// Which select trigger the mouse is over (its own click toggles the menu).
    trigger_hover: Option<Menu>,
    /// Branches of the project (local runs), and the folder they were read from.
    branches: Vec<String>,
    branches_of: String,
    default_branch: Option<String>,
    busy: bool,
    error: Option<String>,
    /// Extra agent CLIs found on PATH (built-in agents are always listed).
    extras: Vec<AgentKind>,
    _load: Task<()>,
    _subs: Vec<Subscription>,
}

impl CreateWorktree {
    pub fn new(deck: Entity<Deck>, prefill: WorktreePrefill, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let d = deck.read(cx);
        let project = prefill
            .project_id
            .filter(|id| d.state.workspaces.iter().any(|w| &w.id == id))
            .unwrap_or_else(|| d.state.active_workspace().id.clone());
        let agent = crate::panes::last_agent(cx);
        let defaults = deck.read(cx).settings.agent(agent);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Branch, new name, #123, link or ACME-412…"));
        let args = cx.new(|cx| {
            InputState::new(window, cx).placeholder("--model sonnet").default_value(defaults.args)
        });
        let base = cx.new(|cx| InputState::new(window, cx).placeholder("Default branch"));
        let location = cx.new(|cx| InputState::new(window, cx).placeholder("Next to the repository"));
        let changed = cx.subscribe(&query, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.error = None;
                cx.notify();
            }
        });
        let mut this = Self {
            deck,
            project,
            host: None,
            mode: Mode::Smart,
            query,
            task: prefill.source,
            agent: Some(agent),
            args,
            show_args: false,
            advanced: false,
            base,
            location,
            chat: false,
            create_more: false,
            menu: None,
            trigger_hover: None,
            branches: Vec::new(),
            branches_of: String::new(),
            default_branch: None,
            busy: false,
            error: None,
            extras: Vec::new(),
            _load: Task::ready(()),
            _subs: vec![changed],
        };
        // A project linked from this dialog ("Link project…") becomes the selected one.
        let mut known = this.deck.read(cx).state.workspaces.iter().map(|w| w.id.clone()).collect::<Vec<_>>();
        let linked = cx.observe(&this.deck, move |this: &mut Self, deck, cx| {
            let added = deck.read(cx).state.workspaces.iter().find(|w| !known.contains(&w.id)).map(|w| w.id.clone());
            if let Some(id) = added {
                // Adopted once: picking another project afterwards is not overridden.
                known.push(id.clone());
                this.project = id;
                this.load_branches(cx);
                cx.notify();
            }
        });
        this._subs.push(linked);
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async { agentdeck_core::agents::detect_agents() }).await;
            let extras: Vec<AgentKind> = found
                .into_iter()
                .filter(|info| info.installed && matches!(info.kind, AgentKind::Other(_)))
                .map(|info| info.kind)
                .collect();
            let _ = this.update(cx, |this, cx| {
                this.extras = extras;
                cx.notify();
            });
        })
        .detach();
        this.load_branches(cx);
        this
    }

    pub fn query_focus(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }

    fn project_path(&self, cx: &App) -> String {
        let d = self.deck.read(cx);
        d.state.workspaces.iter().find(|w| w.id == self.project).map(|w| w.path.clone()).unwrap_or_default()
    }

    /// Branch list and default branch of the project, off the UI thread (local runs only).
    fn load_branches(&mut self, cx: &mut Context<Self>) {
        let path = self.project_path(cx);
        if path == self.branches_of {
            return;
        }
        self.branches_of = path.clone();
        self.branches.clear();
        self.default_branch = None;
        if path.is_empty() {
            return;
        }
        self._load = cx.spawn(async move |this, cx| {
            let dir = path.clone();
            let (branches, default) = cx
                .background_spawn(async move {
                    (branch_worktree::list_branches(&dir).unwrap_or_default(), branch_worktree::default_branch(&dir))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.branches_of == path {
                    this.branches = branches;
                    this.default_branch = default;
                    cx.notify();
                }
            });
        });
    }

    fn set_query(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    /// What the field means in the current mode (also the hint under it).
    fn plan(&self, short: &str, cx: &App) -> Result<Plan, String> {
        if let Some(task) = &self.task {
            return Ok(match task {
                TaskSource::GitHub { kind: GitHubKind::PullRequest, number, .. } => {
                    Plan::GitHub(Some(GitHubKind::PullRequest), *number)
                }
                task => Plan::Task(task.clone()),
            });
        }
        let text = self.query.read(cx).value().trim().to_owned();
        let base = self.base.read(cx).value().trim().to_owned();
        let new = |branch: String| Plan::Start(Start::New { branch, base: base.clone() });
        let auto = || new(format!("agentdeck/{short}"));
        let linear = |key: String| {
            Plan::Task(TaskSource::Linear { key, title: String::new(), url: String::new(), body: None })
        };
        let github = || {
            parse_github(&text)
                .or_else(|| text.parse().ok().filter(|n| *n > 0).map(|n| (None, n)))
                .map(|(kind, n)| Plan::GitHub(kind, n))
                .ok_or_else(|| "GitHub: a pull request or issue number (#123) or its link.".to_owned())
        };
        match self.mode {
            Mode::Smart => match classify(&text, &self.branches) {
                Smart::Empty => Ok(auto()),
                Smart::GitHub(kind, n) => Ok(Plan::GitHub(kind, n)),
                Smart::Linear(key) => Ok(linear(key)),
                Smart::Branch(branch) => Ok(Plan::Start(Start::Existing(branch))),
                Smart::Name(name) if valid_branch(&name) => Ok(new(name)),
                Smart::Name(_) => Err("Type a branch name with letters or digits.".into()),
            },
            Mode::GitHub => github(),
            Mode::Linear => parse_linear(&text).map(linear).ok_or_else(|| "Linear: an issue key like ACME-412, or its link.".into()),
            Mode::Branch if valid_branch(&text) => Ok(Plan::Start(Start::Existing(text))),
            Mode::Branch => Err("Pick a branch from the list.".into()),
            Mode::Name if text.is_empty() => Ok(auto()),
            Mode::Name => match branch_from_name(&text) {
                name if valid_branch(&name) => Ok(new(name)),
                _ => Err("Type a branch name with letters or digits.".into()),
            },
        }
    }

    fn hint(&self, cx: &App) -> String {
        let remote = self.host.is_some();
        match self.plan("<id>", cx) {
            Err(e) => e,
            Ok(Plan::GitHub(..)) if remote => "GitHub needs the project on this computer.".into(),
            Ok(Plan::GitHub(Some(GitHubKind::PullRequest), n)) => format!("Pull request #{n}: its branch, fetched with gh"),
            Ok(Plan::GitHub(Some(GitHubKind::Issue), n)) => format!("Issue #{n}: new branch named from its title"),
            Ok(Plan::GitHub(None, n)) => format!("GitHub #{n}: pull request branch, or a new branch for the issue"),
            Ok(Plan::Task(task)) => format!("New branch {}", task.branch()),
            Ok(Plan::Start(Start::Existing(b))) => format!("Existing branch {b}"),
            Ok(Plan::Start(Start::New { branch, .. })) if branch.starts_with("agentdeck/") => {
                "New agentdeck/… branch (type to name it)".into()
            }
            Ok(Plan::Start(Start::New { branch, .. })) => format!("New branch {branch}"),
        }
    }

    fn pick_agent(&mut self, agent: Option<AgentKind>, window: &mut Window, cx: &mut Context<Self>) {
        self.agent = agent;
        self.menu = None;
        if let Some(kind) = agent {
            let args = self.deck.read(cx).settings.agent(kind).args;
            self.args.update(cx, |s, cx| s.set_value(args, window, cx));
        }
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let short: String = new_id().chars().filter(char::is_ascii_alphanumeric).take(8).collect();
        let plan = match self.plan(&short, cx) {
            Ok(plan) => plan,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let path = self.project_path(cx);
        let target = match &self.host {
            Some(id) => match crate::hosts::get(id, cx) {
                Some(host) => Target::Remote { dir: host.folder.clone(), host },
                None => {
                    self.error = Some("This host was removed: pick another one.".into());
                    cx.notify();
                    return;
                }
            },
            None if path.is_empty() => {
                self.error = Some("This project has no folder: link one, or run on a host.".into());
                cx.notify();
                return;
            }
            None => Target::Local {
                cwd: path,
                parent: self.location.read(cx).value().trim().to_owned(),
                setup: self.deck.read(cx).settings.worktree_setup.clone(),
            },
        };
        let job = Job { plan, target, short, base: self.base.read(cx).value().trim().to_owned() };
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { run_job(job) }).await;
            let _ = this.update_in(cx, |this, window, cx| this.finish(result, window, cx));
        })
        .detach();
    }

    /// Session named after the branch under the project, agent started in the worktree.
    fn finish(&mut self, result: Result<Created, String>, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = false;
        let created = match result {
            Ok(created) => created,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let ws = self.project.clone();
        let host = self.host.as_deref().and_then(|id| crate::hosts::get(id, cx));
        let (tab_id, font_size, local_cwd) = self.deck.update(cx, |d, cx| {
            d.state.select_workspace(&ws);
            let tab_id = d.state.add_tab(Some(new_tab(&created.branch, &ws, None)));
            d.toast(format!("Worktree {} created: {}", created.branch, created.path), cx);
            d.save(cx);
            cx.notify();
            let path = d.state.active_workspace().path.clone();
            (tab_id, d.settings.font_size, if path.is_empty() { d.state.last_cwd.clone() } else { path })
        });
        if let Some(agent) = self.agent {
            let title = created.task.as_ref().map_or_else(|| agent_name(agent).to_owned(), TaskSource::label);
            // Over ssh the local folder only hosts the ssh process; the agent runs in the remote worktree.
            let cwd = if host.is_some() { local_cwd } else { created.path.clone() };
            let mut spec = PaneSpec::new(agent, title, cwd);
            spec.args = split_args(&self.args.read(cx).value());
            spec.font_size = font_size;
            spec.chat = self.chat && host.is_none() && matches!(agent, AgentKind::Claude | AgentKind::Codex);
            if let Some(host) = &host {
                spec.ssh = Some(host.ssh_spec(&created.path));
                spec.host = Some(host.id.clone());
            }
            let prompt = created.task.as_ref().map(TaskSource::prompt);
            start_agent(self.deck.clone(), spec, tab_id, prompt, cx);
        }
        if self.create_more {
            self.task = None;
            self.set_query(String::new(), window, cx);
            cx.notify();
        } else {
            window.close_dialog(cx);
        }
    }

    // ---- rendering --------------------------------------------------------------------------

    fn label(text: &'static str, optional: bool, p: &Palette) -> Div {
        // Fixed height: a label with a button beside it ("Project" + link) lines up with the others.
        div()
            .flex()
            .h(ui(22.0))
            .items_center()
            .gap(ui(6.0))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(hsla(p.muted))
            .child(text)
            .when(optional, |el| {
                el.child(
                    div()
                        .px(ui(5.0))
                        .rounded(ui(RADIUS_SM))
                        .bg(hsla(p.raised))
                        .text_size(ui(10.0))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(hsla(p.subtle))
                        .child("Optional"),
                )
            })
    }

    /// A select's button: content, then up/down chevrons. Toggles `menu`.
    fn trigger(&self, menu: Menu, id: &'static str, content: impl IntoElement, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette(cx);
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .h(ui(32.0))
            .px_2()
            .rounded(ui(RADIUS_MD))
            .border_1()
            .border_color(hsla(if self.menu == Some(menu) { p.line_strong } else { p.line }))
            .bg(hsla(p.raised))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(p.hover)))
            .child(div().flex().flex_1().min_w_0().items_center().gap_2().overflow_hidden().child(content))
            .child(div().text_size(ui(13.0)).text_color(hsla(p.subtle)).child(IconName::ChevronsUpDown))
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| {
                if *hovered {
                    this.trigger_hover = Some(menu);
                } else if this.trigger_hover == Some(menu) {
                    this.trigger_hover = None;
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.menu = if this.menu == Some(menu) { None } else { Some(menu) };
                cx.notify();
            }))
    }

    /// The open select's list, drawn above everything under its trigger.
    fn popup(&self, menu: Menu, rows: Vec<AnyElement>, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        deferred(
            div()
                .absolute()
                .top(ui(36.0))
                .left_0()
                .right_0()
                .id(SharedString::from(format!("wt-menu-{menu:?}")))
                .max_h(ui(260.0))
                .flex()
                .flex_col()
                .p_1()
                .rounded(ui(RADIUS_LG))
                .border_1()
                .border_color(hsla(p.line))
                .bg(hsla(p.panel))
                .shadow_lg()
                .on_mouse_down_out(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    // A press on this menu's own trigger is left to its toggle.
                    if this.trigger_hover != Some(menu) {
                        this.menu = None;
                        cx.notify();
                    }
                }))
                .overflow_y_scroll()
                .children(rows),
        )
        // The dialog surface itself paints in gpui-kit's popup layer (POPUP_PRIORITY = 100): menus
        // inside it go just above, or they open behind the dialog.
        .with_priority(101)
    }

    fn row(id: impl Into<ElementId>, selected: bool, p: &Palette) -> Stateful<Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .h(ui(30.0))
            .px_2()
            .rounded(ui(RADIUS_MD))
            .whitespace_nowrap()
            .cursor_pointer()
            .text_color(hsla(if selected { p.fg } else { p.muted }))
            .when(selected, |el| el.bg(hsla(p.raised)))
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
    }

    /// Project glyph: a plain folder icon.
    fn dot(_id: &str) -> Div {
        div().flex_shrink_0().text_size(ui(14.0)).child(IconName::Folder)
    }

    /// Opens the usual "Link project" dialog; the project it adds is selected here.
    fn link_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        crate::orchestrator::open_link_workspace(self.deck.clone(), window, cx);
        cx.notify();
    }

    /// Small folder+ button beside the "Project" label.
    fn link_button(&self, p: &Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("wt-link-project")
            .flex()
            .items_center()
            .justify_center()
            .size(ui(20.0))
            .rounded(ui(RADIUS_MD))
            .text_size(ui(13.0))
            .text_color(hsla(p.subtle))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(p.hover)).text_color(hsla(p.fg)))
            .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Link project").build(window, cx))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.link_project(window, cx)))
            .child(IconName::FolderPlus)
    }

    fn project_select(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let workspaces = self.deck.read(cx).state.workspaces.clone();
        let current = workspaces.iter().find(|w| w.id == self.project);
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(Self::dot(&self.project))
            .child(div().truncate().child(current.map_or_else(|| "Pick a project".to_owned(), |w| w.name.clone())));
        let mut el = div().relative().flex_1().min_w_0().child(self.trigger(Menu::Project, "wt-project", content, cx));
        if self.menu == Some(Menu::Project) {
            let rows = workspaces
                .iter()
                .map(|w| {
                    let id = w.id.clone();
                    Self::row(SharedString::from(format!("wt-project-{}", w.id)), w.id == self.project, &p)
                        .child(Self::dot(&w.id))
                        .child(div().truncate().child(w.name.clone()))
                        .when(w.path.is_empty(), |el| {
                            el.child(div().ml_auto().text_xs().text_color(hsla(p.subtle)).child("no folder"))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project = id.clone();
                            this.menu = None;
                            this.load_branches(cx);
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .chain(std::iter::once(
                    Self::row(SharedString::from("wt-project-link"), false, &p)
                        .text_color(hsla(p.muted))
                        .child(div().text_size(ui(13.0)).child(IconName::FolderPlus))
                        .child("Link project…")
                        .on_click(cx.listener(|this, _, window, cx| this.link_project(window, cx)))
                        .into_any_element(),
                ))
                .collect();
            el = el.child(self.popup(Menu::Project, rows, cx));
        }
        el
    }

    fn host_select(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let hosts = crate::hosts::list(cx);
        let name = self
            .host
            .as_deref()
            .and_then(|id| hosts.iter().find(|h| h.id == id))
            .map_or_else(|| "This computer".to_owned(), |h| h.name.clone());
        let content = div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(div().text_size(ui(13.0)).text_color(hsla(p.subtle)).child(if self.host.is_some() {
                IconName::Server
            } else {
                IconName::Laptop
            }))
            .child(div().truncate().child(name));
        let mut el = div().relative().flex_1().min_w_0().child(self.trigger(Menu::Host, "wt-host", content, cx));
        if self.menu == Some(Menu::Host) {
            let options = std::iter::once((None, "This computer".to_owned(), IconName::Laptop))
                .chain(hosts.into_iter().map(|h| (Some(h.id), h.name, IconName::Server)));
            let rows = options
                .enumerate()
                .map(|(i, (id, name, icon))| {
                    let selected = self.host == id;
                    Self::row(("wt-host", i), selected, &p)
                        .child(div().text_size(ui(13.0)).child(icon))
                        .child(div().truncate().child(name))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.host = id.clone();
                            this.menu = None;
                            this.error = None;
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect();
            el = el.child(self.popup(Menu::Host, rows, cx));
        }
        el
    }

    fn agent_select(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let content = match self.agent {
            Some(kind) => div().flex().items_center().gap_2().child(agent_logo(kind, 14.0, None, &p)).child(agent_name(kind)),
            None => div().text_color(hsla(p.subtle)).child("No agent"),
        };
        let mut el = div().relative().w_full().child(self.trigger(Menu::Agent, "wt-agent", content, cx));
        if self.menu == Some(Menu::Agent) {
            let options = AGENT_KINDS.into_iter().chain(self.extras.iter().copied()).map(Some).chain([None]);
            let rows = options
                .enumerate()
                .map(|(i, kind)| {
                    let row = Self::row(("wt-agent", i), self.agent == kind, &p);
                    let row = match kind {
                        Some(kind) => row.child(agent_logo(kind, 14.0, None, &p)).child(agent_name(kind)),
                        None => row.child(div().text_size(ui(13.0)).child(IconName::X)).child("No agent"),
                    };
                    row.on_click(cx.listener(move |this, _, window, cx| this.pick_agent(kind, window, cx)))
                        .into_any_element()
                })
                .collect();
            el = el.child(self.popup(Menu::Agent, rows, cx));
        }
        el
    }

    fn mode_tabs(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let remote = self.host.is_some();
        div()
            .flex()
            .gap_1()
            .p(ui(2.0))
            .rounded(ui(RADIUS_MD))
            .bg(hsla(p.raised))
            .children(MODES.iter().map(|(mode, name, icon)| {
                let (mode, selected) = (*mode, self.mode == *mode && self.task.is_none());
                let disabled = remote && mode == Mode::GitHub;
                div()
                    .id(SharedString::from(format!("wt-mode-{name}")))
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(ui(5.0))
                    .h(ui(26.0))
                    .rounded(ui(RADIUS_MD))
                    .text_xs()
                    .text_color(hsla(if selected { p.fg } else { p.muted }))
                    .when(selected, |el| el.bg(hsla(p.panel)).shadow_sm())
                    .when(disabled, |el| {
                        el.opacity(0.4).tooltip(|window, cx| {
                            Tooltip::new("GitHub works on this computer only (gh runs here)").build(window, cx)
                        })
                    })
                    .when(!disabled && !selected, |el| el.cursor_pointer().hover(|s| s.text_color(hsla(p.fg))))
                    .when(!disabled, |el| {
                        el.on_click(cx.listener(move |this, _, _, cx| {
                            this.mode = mode;
                            this.task = None;
                            this.error = None;
                            cx.notify();
                        }))
                    })
                    .child(div().text_size(ui(12.0)).child(icon.clone()))
                    .child(*name)
            }))
    }

    /// The prefilled task: provider icon, key and title, open link, clear.
    fn task_chip(&self, task: &TaskSource, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let icon = match task {
            TaskSource::GitHub { .. } => IconName::Github,
            TaskSource::Linear { .. } => IconName::SquareKanban,
        };
        let url = task.url().to_owned();
        div()
            .flex()
            .items_center()
            .gap_2()
            .h(ui(32.0))
            .pl_2()
            .pr_1()
            .rounded(ui(RADIUS_MD))
            .border_1()
            .border_color(hsla(p.line_strong))
            .bg(hsla(p.raised))
            .child(div().text_size(ui(14.0)).text_color(hsla(p.muted)).child(icon))
            .child(div().flex_shrink_0().font_family(FONT_MONO).text_xs().text_color(hsla(p.muted)).child(task.label()))
            .child(div().flex_1().min_w_0().truncate().child(task.title().to_owned()))
            .when(!url.is_empty(), |el| {
                el.child(
                    small_icon("wt-task-open", IconName::ExternalLink, &p)
                        .tooltip(|window, cx| Tooltip::new("Open the task").build(window, cx))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            })
            .child(
                small_icon("wt-task-clear", IconName::X, &p)
                    .tooltip(|window, cx| Tooltip::new("Clear the task").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.task = None;
                        cx.notify();
                    })),
            )
    }

    fn branch_list(&self, cx: &mut Context<Self>) -> Option<Div> {
        if self.mode != Mode::Branch || self.task.is_some() || self.host.is_some() {
            return None;
        }
        let p = palette(cx);
        let query = self.query.read(cx).value().to_string();
        let rows: Vec<AnyElement> = filter_branches(&self.branches, &query, BRANCH_ROWS)
            .into_iter()
            .enumerate()
            .map(|(i, branch)| {
                let name = branch.clone();
                Self::row(("wt-branch", i), query.trim() == branch.as_str(), &p)
                    .h(ui(26.0))
                    .text_xs()
                    .child(div().text_size(ui(12.0)).text_color(hsla(p.subtle)).child(IconName::GitBranch))
                    .child(div().truncate().font_family(FONT_MONO).child(branch.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.set_query(name.clone(), window, cx)))
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        Some(
            div()
                .flex()
                .flex_col()
                .children(rows)
                .when(empty, |el| el.child(div().px_2().text_xs().text_color(hsla(p.subtle)).child("No matching branch."))),
        )
    }

    fn advanced_section(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let chat_ok = self.host.is_none() && matches!(self.agent, Some(AgentKind::Claude | AgentKind::Codex));
        let field = |input: &Entity<InputState>| div().font_family(FONT_MONO).text_xs().child(Input::new(input).h(ui(30.0)));
        let mode = |id: &'static str, text: &'static str, chat: bool, cx: &mut Context<Self>| {
            let selected = self.chat == chat;
            div()
                .id(id)
                .flex_1()
                .flex()
                .justify_center()
                .py_1()
                .rounded(ui(RADIUS_MD))
                .cursor_pointer()
                .text_xs()
                .when(selected, |el| el.bg(hsla(p.panel)).text_color(hsla(p.fg)))
                .when(!selected, |el| el.text_color(hsla(p.muted)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.chat = chat;
                    cx.notify();
                }))
                .child(text)
        };
        let base_hint = self.default_branch.clone().unwrap_or_else(|| "the default branch".into());
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(ui(6.0))
                    .child(Self::label("Base branch", false, &p))
                    .child(field(&self.base))
                    .child(div().text_xs().text_color(hsla(p.subtle)).child(format!("New branches start from {base_hint}."))),
            )
            .when(self.host.is_none(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui(6.0))
                        .child(Self::label("Worktree folder location", false, &p))
                        .child(field(&self.location)),
                )
            })
            .when(chat_ok, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui(6.0))
                        .child(Self::label("Mode", false, &p))
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .p(ui(2.0))
                                .rounded(ui(RADIUS_MD))
                                .bg(hsla(p.raised))
                                .child(mode("wt-terminal", "Terminal", false, cx))
                                .child(mode("wt-chat", "Chat", true, cx)),
                        ),
                )
            })
    }
}

fn small_icon(id: &'static str, icon: IconName, p: &Palette) -> Stateful<Div> {
    let (hover, fg) = (hsla(p.hover), hsla(p.fg));
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(ui(22.0))
        .rounded(ui(RADIUS_MD))
        .text_size(ui(13.0))
        .text_color(hsla(p.subtle))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .child(icon)
}

impl Render for CreateWorktree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let hint = self.hint(cx);
        let task = self.task.clone();
        let create_more = self.create_more;
        let advanced = self.advanced;
        div()
            .flex()
            .flex_col()
            .w_full()
            .text_size(ui(13.0))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key == "enter" && event.keystroke.modifiers.control {
                    cx.stop_propagation();
                    this.submit(window, cx);
                } else if key == "escape" && this.menu.is_some() {
                    cx.stop_propagation();
                    this.menu = None;
                    cx.notify();
                } else if key == "escape" {
                    cx.stop_propagation();
                    window.close_dialog(cx);
                }
            }))
            // Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_4()
                    .pr_3()
                    .pl_4()
                    .child(div().text_size(ui(15.0)).font_weight(FontWeight::MEDIUM).child("Create worktree"))
                    .child(crate::panes::close_button("wt-close", "Close (Esc)").on_click(|window, cx| window.close_dialog(cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_4()
                    // Project + Run on
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(ui(6.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(Self::label("Project", false, &p))
                                            .child(self.link_button(&p, cx)),
                                    )
                                    .child(self.project_select(cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(ui(6.0))
                                    .child(Self::label("Run on", false, &p))
                                    .child(self.host_select(cx)),
                            ),
                    )
                    // Create from
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui(6.0))
                            .child(Self::label("Create From", true, &p))
                            .child(self.mode_tabs(cx))
                            .map(|el| match &task {
                                Some(task) => el.child(self.task_chip(task, cx)),
                                None => el.child(
                                    Input::new(&self.query)
                                        .h(ui(32.0))
                                        .prefix(Icon::new(IconName::Search).text_color(hsla(p.subtle))),
                                ),
                            })
                            .children(self.branch_list(cx))
                            .child(div().text_xs().text_color(hsla(p.subtle)).truncate().child(hint)),
                    )
                    // Agent
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(ui(6.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(Self::label("Agent", true, &p))
                                    .when(self.agent.is_some(), |el| {
                                        el.child(
                                            small_icon("wt-args", IconName::SlidersHorizontal, &p)
                                                .when(self.show_args, |el| el.text_color(hsla(p.fg)))
                                                .tooltip(|window, cx| Tooltip::new("Agent arguments").build(window, cx))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.show_args = !this.show_args;
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            )
                            .child(self.agent_select(cx))
                            .when(self.show_args && self.agent.is_some(), |el| {
                                el.child(div().font_family(FONT_MONO).text_xs().child(Input::new(&self.args).h(ui(30.0))))
                            }),
                    )
                    // Advanced
                    .child(
                        div()
                            .id("wt-advanced")
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .cursor_pointer()
                            .hover(|s| s.text_color(hsla(p.fg)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.advanced = !this.advanced;
                                cx.notify();
                            }))
                            .child("Advanced")
                            .child(div().text_size(ui(12.0)).child(if advanced { IconName::ChevronDown } else { IconName::ChevronRight })),
                    )
                    .when(advanced, |el| el.child(self.advanced_section(cx)))
                    .when_some(self.error.clone(), |el, error| {
                        el.child(
                            div()
                                .rounded(ui(RADIUS_MD))
                                .bg(hsla_a(p.bad, 0.1))
                                .px_3()
                                .py_2()
                                .text_xs()
                                .text_color(hsla(p.bad))
                                .child(error),
                        )
                    }),
            )
            // Footer
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .pb_4()
                    .child(
                        div()
                            .id("wt-more")
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(hsla(p.muted))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.create_more = !this.create_more;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .w(ui(26.0))
                                    .h(ui(15.0))
                                    .p(ui(2.0))
                                    .rounded_full()
                                    .bg(hsla(if create_more { p.accent } else { p.line_strong }))
                                    .when(create_more, |el| el.justify_end())
                                    .child(div().size(ui(11.0)).rounded_full().bg(hsla(p.fg))),
                            )
                            .child("Create more"),
                    )
                    .child(
                        div()
                            .id("wt-create")
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(ui(30.0))
                            .px_3()
                            .rounded(ui(RADIUS_MD))
                            .bg(hsla(p.fg))
                            .text_color(hsla(p.base))
                            .font_weight(FontWeight::MEDIUM)
                            .when(self.busy, |el| el.opacity(0.6))
                            .when(!self.busy, |el| el.cursor_pointer().hover(|s| s.opacity(0.9)))
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
                            .child(if self.busy { "Creating…" } else { "Create worktree" })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(ui(2.0))
                                    .text_xs()
                                    .opacity(0.6)
                                    .child("Ctrl")
                                    .child(div().text_size(ui(11.0)).child(IconName::CornerDownLeft)),
                            ),
                    ),
            )
    }
}
