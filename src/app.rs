//! Application state and the message/update loop (Elm style).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;
use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::ApiClient;
use crate::cup::CupClient;
use crate::keys;
use crate::model::*;
use crate::text;

pub const DETAIL_CACHE_TTL: Duration = Duration::from_secs(45);
const DETAIL_DEBOUNCE: Duration = Duration::from_millis(180);

pub enum Msg {
    Term(Event),
    Tick,
    Auth(Result<User, String>),
    Spaces(Result<Vec<Space>, String>),
    SpaceChildren { space_id: String, result: Result<SpaceChildren, String> },
    Tasks { key: String, page: u32, result: Result<(Vec<TaskSummary>, bool), String> },
    Activity { id: String, result: Result<(TaskDetail, Vec<Comment>), String> },
    Subtasks { id: String, result: Result<Vec<TaskSummary>, String> },
    Members(Result<Vec<User>, String>),
    ActionDone {
        label: String,
        task_id: Option<String>,
        result: Result<String, String>,
        created: bool,
    },
}

/// Side effects the main loop must perform outside the event loop.
pub enum Effect {
    Editor { purpose: EditorPurpose, initial: String },
}

#[derive(Clone, Debug)]
pub enum EditorPurpose {
    Comment { task_id: String },
    Description { task_id: String },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Sidebar,
    Tasks,
    Detail,
}

pub enum Mode {
    Normal,
    Help,
    Input(InputState),
    Picker(PickerState),
}

pub struct InputState {
    pub title: String,
    pub value: String,
    /// Cursor position in chars.
    pub cursor: usize,
    pub purpose: InputPurpose,
    pub hint: String,
}

#[derive(Clone, Debug)]
pub enum InputPurpose {
    FilterTasks,
    FilterSidebar,
    Search { scope: SearchScope },
    NewTask { list_id: String, parent: Option<String> },
    Rename { task_id: String },
    DueDate { task_id: String },
    Tags { task_id: String },
}

pub struct PickerState {
    pub title: String,
    pub items: Vec<PickerItem>,
    pub filter: String,
    pub selected: usize,
    pub purpose: PickerPurpose,
    /// Multi-select: Tab toggles, Enter applies the checked set.
    pub multi: bool,
    /// Lower-cased item values that are checked.
    pub checked: HashSet<String>,
}

impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<PickerItem>, selected: usize, purpose: PickerPurpose) -> Self {
        Self { title: title.into(), items, filter: String::new(), selected, purpose, multi: false, checked: HashSet::new() }
    }

    pub fn is_checked(&self, item: &PickerItem) -> bool {
        self.checked.contains(&item.value.to_lowercase())
    }

    pub fn filtered(&self) -> Vec<usize> {
        let mut v: Vec<(i64, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| text::fuzzy_score(&it.label, &self.filter).map(|s| (s, i)))
            .collect();
        if !self.filter.is_empty() {
            v.sort_by_key(|(s, i)| (*s, *i));
        }
        v.into_iter().map(|(_, i)| i).collect()
    }
}

pub struct PickerItem {
    pub label: String,
    pub value: String,
    pub color: Option<Color>,
    pub hint: String,
}

#[derive(Clone, Debug)]
pub enum PickerPurpose {
    Status { task_id: String },
    Priority { task_id: String },
    Assignee { task_id: String },
    Sort,
    FilterMenu,
    FilterStatus,
    FilterPriority,
    FilterAssignee,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode {
    Status,
    Priority,
    Due,
    Name,
    Default,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Status => "status",
            SortMode::Priority => "priority",
            SortMode::Due => "due",
            SortMode::Name => "name",
            SortMode::Default => "clickup",
        }
    }
    pub const ALL: [SortMode; 5] = [
        SortMode::Status,
        SortMode::Priority,
        SortMode::Due,
        SortMode::Name,
        SortMode::Default,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub enum RowKind {
    Header,
    Virtual(TaskSource),
    Space(String),
    Folder { space_id: String, id: String },
    List {
        space_id: String,
        id: String,
        name: String,
        folder_name: Option<String>,
        space_name: String,
    },
}

#[derive(Clone, Debug)]
pub struct SidebarRow {
    pub kind: RowKind,
    pub label: String,
    pub depth: u8,
    pub expandable: bool,
    pub expanded: bool,
    pub loading: bool,
    pub count: Option<i64>,
    pub active: bool,
}

impl SidebarRow {
    pub fn selectable(&self) -> bool {
        !matches!(self.kind, RowKind::Header)
    }
}

#[derive(Clone, Debug)]
pub struct TaskRow {
    /// Task id, or `#group:<status>` for a header row.
    pub id: String,
    pub depth: u8,
    /// Group header row: (status name, number of top-level tasks).
    pub header: Option<(String, usize)>,
    /// Direct subtasks this row has in the current view (0 for leaves).
    pub child_count: usize,
    /// Children (or group members) are hidden.
    pub collapsed: bool,
}

impl TaskRow {
    pub fn is_header(&self) -> bool {
        self.header.is_some()
    }
}

/// Tasks loaded so far for one source key, page by page.
pub struct TaskPages {
    pub tasks: Vec<TaskSummary>,
    pub next_page: u32,
    pub has_more: bool,
}

/// Vim-style visual selection in the detail pane.
#[derive(Clone, Copy, Debug)]
pub struct Visual {
    pub anchor: (usize, usize),
    pub linewise: bool,
}

pub struct CachedDetail {
    pub task: TaskDetail,
    pub comments: Vec<Comment>,
    pub at: Instant,
}

#[derive(Default, Clone, Copy)]
pub struct LayoutRects {
    pub sidebar: Rect,
    pub tasks: Rect,
    pub detail: Rect,
}

pub struct App {
    pub client: Arc<CupClient>,
    /// Direct API access (cup's token). None → everything goes through cup.
    pub api: Option<Arc<ApiClient>>,
    pub api_error: Option<String>,
    pub tx: UnboundedSender<Msg>,
    pub effects: Vec<Effect>,
    pub should_quit: bool,
    pub me: Option<User>,

    // sidebar
    pub spaces: Vec<Space>,
    pub spaces_loading: bool,
    pub spaces_error: Option<String>,
    pub space_children: HashMap<String, SpaceChildren>,
    pub loading_spaces: HashSet<String>,
    pub expanded_spaces: HashSet<String>,
    pub collapsed_folders: HashSet<String>,
    pub sidebar_state: ListState,
    pub sidebar_filter: String,
    pub show_sidebar: bool,

    // tasks
    pub source: Option<TaskSource>,
    pub tasks_cache: HashMap<String, TaskPages>,
    pub tasks_loading: HashSet<String>,
    pub tasks_error: HashMap<String, String>,
    pub include_closed: bool,
    pub filters: TaskFilters,
    pub task_filter: String,
    pub sort: SortMode,
    pub rows: Vec<TaskRow>,
    pub table_state: TableState,
    pub pending_select: Option<String>,
    /// Parent tasks whose subtasks are shown (parents start collapsed).
    pub expanded_tasks: HashSet<String>,

    // detail
    pub detail_open: bool,
    pub detail_id: Option<String>,
    pub details: HashMap<String, CachedDetail>,
    pub subtasks: HashMap<String, Vec<TaskSummary>>,
    pub detail_loading: HashSet<String>,
    pub detail_error: HashMap<String, String>,
    pub detail_scroll: u16,
    pub detail_max_scroll: u16,
    pub pending_detail: Option<(String, Instant)>,
    /// Cursor in the detail pane: (visual row, char column).
    pub detail_cursor: (usize, usize),
    /// Column the user last asked for; j/k try to return to it (like vim).
    pub detail_want_col: usize,
    /// Active visual selection.
    pub visual: Option<Visual>,
    /// Plain text of each wrapped row, as last rendered.
    pub detail_rows_plain: Vec<String>,
    /// Logical (unwrapped) line index of each wrapped row; rows sharing one are re-joined with a space on copy.
    pub detail_row_line: Vec<usize>,
    pub detail_view_height: u16,
    /// Text area of the detail pane, for mouse clicks.
    pub detail_inner: Rect,
    /// A `g` was pressed in the detail pane and `gg` may follow.
    pub pending_g: bool,

    // misc
    pub members: Vec<User>,
    pub status_colors: HashMap<String, Color>,
    pub focus: Focus,
    pub mode: Mode,
    pub toast: Option<Toast>,
    pub inflight: usize,
    pub spinner: usize,
    pub layout: LayoutRects,
    pub narrow: bool,
}

impl App {
    pub fn new(client: Arc<CupClient>, api: Result<ApiClient, String>, tx: UnboundedSender<Msg>) -> Self {
        let mut sidebar_state = ListState::default();
        sidebar_state.select(Some(1));
        let (api, api_error) = match api {
            Ok(a) => (Some(Arc::new(a)), None),
            Err(e) => (None, Some(e)),
        };
        Self {
            client,
            api,
            api_error,
            tx,
            effects: Vec::new(),
            should_quit: false,
            me: None,
            spaces: Vec::new(),
            spaces_loading: false,
            spaces_error: None,
            space_children: HashMap::new(),
            loading_spaces: HashSet::new(),
            expanded_spaces: HashSet::new(),
            collapsed_folders: HashSet::new(),
            sidebar_state,
            sidebar_filter: String::new(),
            show_sidebar: true,
            source: None,
            tasks_cache: HashMap::new(),
            tasks_loading: HashSet::new(),
            tasks_error: HashMap::new(),
            include_closed: false,
            filters: TaskFilters::default(),
            task_filter: String::new(),
            sort: SortMode::Status,
            rows: Vec::new(),
            table_state: TableState::default(),
            pending_select: None,
            expanded_tasks: HashSet::new(),
            detail_open: false,
            detail_id: None,
            details: HashMap::new(),
            subtasks: HashMap::new(),
            detail_loading: HashSet::new(),
            detail_error: HashMap::new(),
            detail_scroll: 0,
            detail_max_scroll: 0,
            pending_detail: None,
            detail_cursor: (0, 0),
            detail_want_col: 0,
            visual: None,
            detail_rows_plain: Vec::new(),
            detail_row_line: Vec::new(),
            detail_view_height: 0,
            detail_inner: Rect::default(),
            pending_g: false,
            members: Vec::new(),
            status_colors: HashMap::new(),
            focus: Focus::Sidebar,
            mode: Mode::Normal,
            toast: None,
            inflight: 0,
            spinner: 0,
            layout: LayoutRects::default(),
            narrow: false,
        }
    }

    pub fn init(&mut self) {
        if let Some(e) = self.api_error.clone() {
            self.toast(format!("Direct API unavailable ({e}); lists via cup: no assignees, no paging"), ToastKind::Error);
        }
        let c = self.client.clone();
        self.spawn(async move { Msg::Auth(c.auth().await) });
        self.load_spaces();
        let c = self.client.clone();
        self.spawn(async move { Msg::Members(c.members().await) });
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn spawn<F>(&mut self, fut: F)
    where
        F: Future<Output = Msg> + Send + 'static,
    {
        self.inflight += 1;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(fut.await);
        });
    }

    fn done(&mut self) {
        self.inflight = self.inflight.saturating_sub(1);
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast { text: text.into(), kind, at: Instant::now() });
    }

    // ------------------------------------------------------------------
    // Update
    // ------------------------------------------------------------------

    /// Returns true when the screen should be redrawn.
    pub fn update(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Term(ev) => match ev {
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    keys::handle_key(self, k);
                    true
                }
                Event::Mouse(m) => {
                    keys::handle_mouse(self, m);
                    true
                }
                Event::Resize(_, _) => true,
                _ => false,
            },
            Msg::Tick => self.on_tick(),
            Msg::Auth(r) => {
                self.done();
                match r {
                    Ok(u) => self.me = Some(u),
                    Err(e) => self.toast(format!("cup auth failed: {e}"), ToastKind::Error),
                }
                true
            }
            Msg::Spaces(r) => {
                self.done();
                self.spaces_loading = false;
                match r {
                    Ok(spaces) => {
                        for sp in &spaces {
                            for st in &sp.statuses {
                                if let Some(c) = st.color.as_deref().and_then(text::hex_color) {
                                    self.status_colors.insert(st.status.to_lowercase(), c);
                                }
                            }
                        }
                        self.spaces = spaces;
                        self.spaces_error = None;
                        self.clamp_sidebar();
                    }
                    Err(e) => {
                        self.spaces_error = Some(e.clone());
                        self.toast(format!("spaces: {e}"), ToastKind::Error);
                    }
                }
                true
            }
            Msg::SpaceChildren { space_id, result } => {
                self.done();
                self.loading_spaces.remove(&space_id);
                match result {
                    Ok(ch) => {
                        self.space_children.insert(space_id, ch);
                    }
                    Err(e) => self.toast(format!("space: {e}"), ToastKind::Error),
                }
                self.clamp_sidebar();
                true
            }
            Msg::Tasks { key, page, result } => {
                self.done();
                self.tasks_loading.remove(&key);
                match result {
                    Ok((tasks, has_more)) => {
                        self.tasks_error.remove(&key);
                        for t in &tasks {
                            if let Some(c) = t.status_color.as_deref().and_then(text::hex_color) {
                                self.status_colors.insert(t.status.to_lowercase(), c);
                            }
                        }
                        let entry = self
                            .tasks_cache
                            .entry(key.clone())
                            .or_insert_with(|| TaskPages { tasks: Vec::new(), next_page: 0, has_more: false });
                        if page == 0 {
                            entry.tasks = tasks;
                        } else {
                            let seen: HashSet<String> = entry.tasks.iter().map(|t| t.id.clone()).collect();
                            entry.tasks.extend(tasks.into_iter().filter(|t| !seen.contains(&t.id)));
                        }
                        entry.next_page = page + 1;
                        entry.has_more = has_more;
                    }
                    Err(e) => {
                        self.tasks_error.insert(key.clone(), e.clone());
                        self.toast(format!("tasks: {e}"), ToastKind::Error);
                    }
                }
                let is_current = self.current_key().as_deref() == Some(key.as_str());
                if is_current {
                    self.rebuild_rows();
                    // keep pulling pages in the background until the list is complete,
                    // so status groups are never half-filled
                    self.load_next_page();
                }
                true
            }
            Msg::Activity { id, result } => {
                self.done();
                self.detail_loading.remove(&id);
                match result {
                    Ok((task, comments)) => {
                        if let Some(st) = &task.status {
                            if let Some(c) = st.color.as_deref().and_then(text::hex_color) {
                                self.status_colors.insert(st.status.to_lowercase(), c);
                            }
                        }
                        let fetch_subs = task.parent.is_none();
                        self.detail_error.remove(&id);
                        self.details.insert(id.clone(), CachedDetail { task, comments, at: Instant::now() });
                        if fetch_subs && !self.has_children_in_rows(&id) {
                            let c = self.client.clone();
                            let tid = id.clone();
                            self.spawn(async move {
                                let r = c.subtasks(&tid).await;
                                Msg::Subtasks { id: tid, result: r }
                            });
                        }
                    }
                    Err(e) => {
                        self.detail_error.insert(id.clone(), e.clone());
                        self.toast(format!("task: {e}"), ToastKind::Error);
                    }
                }
                true
            }
            Msg::Subtasks { id, result } => {
                self.done();
                if let Ok(s) = result {
                    self.subtasks.insert(id, s);
                }
                true
            }
            Msg::Members(r) => {
                self.done();
                if let Ok(m) = r {
                    self.members = m;
                }
                false
            }
            Msg::ActionDone { label, task_id, result, created } => {
                self.done();
                match result {
                    Ok(out) => {
                        self.toast(label, ToastKind::Success);
                        if created {
                            if let Some(id) = serde_json::from_str::<serde_json::Value>(&out)
                                .ok()
                                .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
                            {
                                self.pending_select = Some(id);
                            }
                        }
                        if let Some(id) = task_id {
                            self.details.remove(&id);
                            self.subtasks.remove(&id);
                            if self.detail_id.as_deref() == Some(id.as_str()) {
                                self.load_detail(&id, true);
                            }
                        }
                        self.reload_current_tasks();
                    }
                    Err(e) => self.toast(format!("{label} failed: {e}"), ToastKind::Error),
                }
                true
            }
        }
    }

    fn on_tick(&mut self) -> bool {
        let mut redraw = false;
        if self.inflight > 0 {
            self.spinner = (self.spinner + 1) % SPINNER.len();
            redraw = true;
        }
        if let Some(t) = &self.toast {
            let ttl = match t.kind {
                ToastKind::Error => Duration::from_secs(8),
                _ => Duration::from_secs(4),
            };
            if t.at.elapsed() > ttl {
                self.toast = None;
                redraw = true;
            }
        }
        if let Some((id, at)) = &self.pending_detail {
            if at.elapsed() >= DETAIL_DEBOUNCE {
                let id = id.clone();
                self.pending_detail = None;
                self.load_detail(&id, false);
                redraw = true;
            }
        }
        redraw
    }

    // ------------------------------------------------------------------
    // Loading
    // ------------------------------------------------------------------

    pub fn load_spaces(&mut self) {
        if self.spaces_loading {
            return;
        }
        self.spaces_loading = true;
        let c = self.client.clone();
        self.spawn(async move { Msg::Spaces(c.spaces().await) });
    }

    pub fn load_space_children(&mut self, space_id: &str, force: bool) {
        if self.loading_spaces.contains(space_id) || (!force && self.space_children.contains_key(space_id)) {
            return;
        }
        self.loading_spaces.insert(space_id.to_string());
        let c = self.client.clone();
        let sid = space_id.to_string();
        self.spawn(async move {
            let r = c.space_children(&sid).await;
            Msg::SpaceChildren { space_id: sid, result: r }
        });
    }

     /// Cache key of the current source, including server-side filters when
    /// the source is fetched from the API.
    pub fn current_key(&self) -> Option<String> {
        self.source.as_ref().map(|s| self.source_key(s))
    }

    pub fn is_api_source(&self, src: &TaskSource) -> bool {
        self.api.is_some()
            && match src {
                TaskSource::List { .. } => true,
                TaskSource::Assigned => self.me.is_some(),
                _ => false,
            }
    }

    pub fn source_key(&self, src: &TaskSource) -> String {
        let base = src.key(self.include_closed);
        if self.is_api_source(src) {
            format!("{base}|{}", self.filters.server_key())
        } else {
            base
        }
    }

     /// Lists, searches and assigned tasks read best grouped by status; the
    /// inbox is about recency and overdue about dates.
    pub fn default_sort_for(source: &TaskSource) -> SortMode {
        match source {
            TaskSource::Inbox => SortMode::Default,
            TaskSource::Overdue => SortMode::Due,
            _ => SortMode::Status,
        }
    }

    pub fn set_source(&mut self, source: TaskSource) {
        let changed = self.source.as_ref() != Some(&source);
        if changed {
            self.sort = Self::default_sort_for(&source);
        }
        self.source = Some(source);
        if changed {
            self.task_filter.clear();
            self.table_state.select(None);
            *self.table_state.offset_mut() = 0;
        }
        self.load_current_tasks(false);
        self.rebuild_rows();
    }

     pub fn load_current_tasks(&mut self, force: bool) {
        let Some(src) = self.source.clone() else { return };
        let key = self.source_key(&src);
        if self.tasks_loading.contains(&key) || (!force && self.tasks_cache.contains_key(&key)) {
            return;
        }
        self.fetch_page(src, key, 0);
    }

    pub fn load_next_page(&mut self) {
        let Some(src) = self.source.clone() else { return };
        let key = self.source_key(&src);
        let Some(pages) = self.tasks_cache.get(&key) else { return };
        if !pages.has_more || self.tasks_loading.contains(&key) {
            return;
        }
        let next = pages.next_page;
        self.fetch_page(src, key, next);
    }

    pub fn has_more_pages(&self) -> bool {
        self.current_key()
            .and_then(|k| self.tasks_cache.get(&k))
            .map(|p| p.has_more)
            .unwrap_or(false)
    }

    pub fn tasks_loading_now(&self) -> bool {
        self.current_key().map(|k| self.tasks_loading.contains(&k)).unwrap_or(false)
    }

    fn fetch_page(&mut self, src: TaskSource, key: String, page: u32) {
        self.tasks_loading.insert(key.clone());
        let closed = self.include_closed;
        let mut filters = self.filters.clone();
        // ClickUp matches statuses[] exactly; some lists have variants like "to do "
        if !filters.statuses.is_empty() {
            let mut extra = Vec::new();
            for t in self.tasks_cache.values().flat_map(|p| p.tasks.iter()) {
                if filters.statuses.iter().any(|s| s.trim().eq_ignore_ascii_case(t.status.trim()))
                    && !filters.statuses.contains(&t.status)
                    && !extra.contains(&t.status)
                {
                    extra.push(t.status.clone());
                }
            }
            filters.statuses.extend(extra);
        }
        let api = if self.is_api_source(&src) { self.api.clone() } else { None };
        let me = self.me.as_ref().map(|u| u.id_string());
        let cup = self.client.clone();
        self.spawn(async move {
            let result = match (&api, &src) {
                (Some(api), TaskSource::List { id, .. }) => {
                    api.list_tasks(id, page, closed, &filters).await.map(|p| (p.tasks, !p.last_page))
                }
                (Some(api), TaskSource::Assigned) => {
                    let ids = vec![me.unwrap_or_default()];
                    api.team_tasks(page, &ids, closed, &filters).await.map(|p| (p.tasks, !p.last_page))
                }
                _ => cup.tasks(&src, closed).await.map(|t| (t, false)),
            };
            Msg::Tasks { key, page, result }
        });
    }

    pub fn reload_current_tasks(&mut self) {
        self.load_current_tasks(true);
    }

    pub fn load_detail(&mut self, id: &str, force: bool) {
        if self.detail_loading.contains(id) {
            return;
        }
        if !force {
            if let Some(c) = self.details.get(id) {
                if c.at.elapsed() < DETAIL_CACHE_TTL {
                    return;
                }
            }
        }
        self.detail_loading.insert(id.to_string());
        let c = self.client.clone();
        let tid = id.to_string();
        self.spawn(async move {
            let r = c.activity(&tid).await;
            Msg::Activity { id: tid, result: r }
        });
    }

    pub fn hard_refresh(&mut self) {
        self.tasks_cache.clear();
        self.details.clear();
        self.subtasks.clear();
        self.space_children.clear();
        self.load_spaces();
        for sid in self.expanded_spaces.clone() {
            self.load_space_children(&sid, true);
        }
        self.load_current_tasks(true);
        if let Some(id) = self.detail_id.clone() {
            self.load_detail(&id, true);
        }
        self.toast("Refreshing everything…", ToastKind::Info);
    }

    // ------------------------------------------------------------------
    // Sidebar
    // ------------------------------------------------------------------

    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let mut rows = Vec::new();
        let f = self.sidebar_filter.to_lowercase();
        let filtering = !f.is_empty();
        let header = |label: &str| SidebarRow {
            kind: RowKind::Header,
            label: label.to_string(),
            depth: 0,
            expandable: false,
            expanded: false,
            loading: false,
            count: None,
            active: false,
        };
        rows.push(header("MINE"));
        for (src, label) in [
            (TaskSource::Assigned, "My tasks"),
            (TaskSource::Inbox, "Inbox"),
            (TaskSource::Overdue, "Overdue"),
        ] {
            if filtering && !label.to_lowercase().contains(&f) {
                continue;
            }
            rows.push(SidebarRow {
                active: self.source.as_ref() == Some(&src),
                kind: RowKind::Virtual(src),
                label: label.to_string(),
                depth: 0,
                expandable: false,
                expanded: false,
                loading: false,
                count: None,
            });
        }
        rows.push(header("SPACES"));
        if self.spaces_loading && self.spaces.is_empty() {
            rows.push(header("loading…"));
        }
        if let Some(e) = &self.spaces_error {
            if self.spaces.is_empty() {
                rows.push(header(&format!("error: {}", text::truncate(e, 28))));
            }
        }
        for sp in &self.spaces {
            let expanded = self.expanded_spaces.contains(&sp.id) || filtering;
            let sp_match = !filtering || sp.name.to_lowercase().contains(&f);
            let mut child_rows = Vec::new();
            if expanded {
                if let Some(ch) = self.space_children.get(&sp.id) {
                    for folder in &ch.folders {
                        let f_match = !filtering || folder.name.to_lowercase().contains(&f);
                        let f_expanded = !self.collapsed_folders.contains(&folder.id) || filtering;
                        let mut lists = Vec::new();
                        if f_expanded {
                            for l in &folder.lists {
                                if filtering && !f_match && !l.name.to_lowercase().contains(&f) {
                                    continue;
                                }
                                lists.push(self.list_row(sp, l, Some(&folder.name), 2));
                            }
                        }
                        if filtering && !f_match && lists.is_empty() {
                            continue;
                        }
                        child_rows.push(SidebarRow {
                            kind: RowKind::Folder { space_id: sp.id.clone(), id: folder.id.clone() },
                            label: folder.name.clone(),
                            depth: 1,
                            expandable: true,
                            expanded: f_expanded,
                            loading: false,
                            count: Some(folder.lists.len() as i64),
                            active: false,
                        });
                        child_rows.extend(lists);
                    }
                    for l in &ch.lists {
                        if filtering && !sp_match && !l.name.to_lowercase().contains(&f) {
                            continue;
                        }
                        child_rows.push(self.list_row(sp, l, None, 1));
                    }
                    if ch.folders.is_empty() && ch.lists.is_empty() && !filtering {
                        child_rows.push(SidebarRow {
                            kind: RowKind::Header,
                            label: "(empty)".into(),
                            depth: 1,
                            expandable: false,
                            expanded: false,
                            loading: false,
                            count: None,
                            active: false,
                        });
                    }
                }
            }
            if filtering && !sp_match && child_rows.is_empty() {
                continue;
            }
            rows.push(SidebarRow {
                kind: RowKind::Space(sp.id.clone()),
                label: sp.name.clone(),
                depth: 0,
                expandable: true,
                expanded,
                loading: self.loading_spaces.contains(&sp.id),
                count: None,
                active: false,
            });
            rows.extend(child_rows);
        }
        rows
    }

    fn list_row(&self, sp: &Space, l: &ListInfo, folder_name: Option<&str>, depth: u8) -> SidebarRow {
        let active = matches!(&self.source, Some(TaskSource::List { id, .. }) if id == &l.id);
        SidebarRow {
            kind: RowKind::List {
                space_id: sp.id.clone(),
                id: l.id.clone(),
                name: l.name.clone(),
                folder_name: folder_name.map(str::to_string),
                space_name: sp.name.clone(),
            },
            label: l.name.clone(),
            depth,
            expandable: false,
            expanded: false,
            loading: false,
            count: l.task_count,
            active,
        }
    }

    pub fn clamp_sidebar(&mut self) {
        let rows = self.sidebar_rows();
        if rows.is_empty() {
            self.sidebar_state.select(None);
            return;
        }
        let mut sel = self.sidebar_state.selected().unwrap_or(0).min(rows.len() - 1);
        if !rows[sel].selectable() {
            // move to nearest selectable row (down first, then up)
            if let Some(i) = (sel..rows.len()).find(|&i| rows[i].selectable()) {
                sel = i;
            } else if let Some(i) = (0..sel).rev().find(|&i| rows[i].selectable()) {
                sel = i;
            }
        }
        self.sidebar_state.select(Some(sel));
    }

    pub fn sidebar_move(&mut self, delta: i32) {
        let rows = self.sidebar_rows();
        if rows.is_empty() {
            return;
        }
        let mut i = self.sidebar_state.selected().unwrap_or(0) as i32;
        let n = rows.len() as i32;
        let step = if delta >= 0 { 1 } else { -1 };
        let mut remaining = delta.abs();
        while remaining > 0 {
            let mut j = i + step;
            while j >= 0 && j < n && !rows[j as usize].selectable() {
                j += step;
            }
            if j < 0 || j >= n {
                break;
            }
            i = j;
            remaining -= 1;
        }
        self.sidebar_state.select(Some(i as usize));
    }

    pub fn sidebar_jump(&mut self, to_end: bool) {
        let rows = self.sidebar_rows();
        let idx = if to_end {
            (0..rows.len()).rev().find(|&i| rows[i].selectable())
        } else {
            (0..rows.len()).find(|&i| rows[i].selectable())
        };
        if idx.is_some() {
            self.sidebar_state.select(idx);
        }
    }

    pub fn sidebar_selected_row(&self) -> Option<SidebarRow> {
        let rows = self.sidebar_rows();
        rows.get(self.sidebar_state.selected()?).cloned()
    }

    /// Enter on a sidebar row.
    pub fn sidebar_activate(&mut self, toggle_only: bool) {
        let Some(row) = self.sidebar_selected_row() else { return };
        match row.kind {
            RowKind::Header => {}
            RowKind::Virtual(src) => {
                self.set_source(src);
                if !toggle_only {
                    self.focus = Focus::Tasks;
                }
            }
            RowKind::Space(id) => {
                if self.expanded_spaces.contains(&id) && !toggle_only {
                    self.expanded_spaces.remove(&id);
                } else {
                    self.expanded_spaces.insert(id.clone());
                    self.load_space_children(&id, false);
                }
                self.clamp_sidebar();
            }
            RowKind::Folder { id, .. } => {
                if self.collapsed_folders.contains(&id) {
                    self.collapsed_folders.remove(&id);
                } else if !toggle_only {
                    self.collapsed_folders.insert(id);
                }
                self.clamp_sidebar();
            }
            RowKind::List { space_id, id, name, folder_name, space_name } => {
                self.set_source(TaskSource::List {
                    id,
                    name,
                    space_id: Some(space_id),
                    folder_name,
                    space_name: Some(space_name),
                });
                if !toggle_only {
                    self.focus = Focus::Tasks;
                }
            }
        }
    }

    /// Left / h on the sidebar: collapse, or jump to the parent row.
    pub fn sidebar_collapse_or_parent(&mut self) {
        let rows = self.sidebar_rows();
        let Some(sel) = self.sidebar_state.selected() else { return };
        let Some(row) = rows.get(sel) else { return };
        match &row.kind {
            RowKind::Space(id) if row.expanded => {
                self.expanded_spaces.remove(id);
                self.clamp_sidebar();
            }
            RowKind::Folder { id, .. } if row.expanded => {
                self.collapsed_folders.insert(id.clone());
                self.clamp_sidebar();
            }
            _ => {
                let depth = row.depth;
                if let Some(p) = (0..sel).rev().find(|&i| rows[i].depth < depth && rows[i].selectable()) {
                    self.sidebar_state.select(Some(p));
                }
            }
        }
    }

    /// Right / l on the sidebar: expand, or step into the first child.
    pub fn sidebar_expand_or_child(&mut self) {
        let Some(row) = self.sidebar_selected_row() else { return };
        if row.expandable && !row.expanded {
            self.sidebar_activate(true);
        } else if row.expandable {
            self.sidebar_move(1);
        } else {
            self.sidebar_activate(false);
        }
    }

    // ------------------------------------------------------------------
    // Tasks table
    // ------------------------------------------------------------------

     pub fn current_tasks(&self) -> &[TaskSummary] {
        self.current_key()
            .and_then(|k| self.tasks_cache.get(&k))
            .map(|p| p.tasks.as_slice())
            .unwrap_or(&[])
    }

    pub fn task_by_id(&self, id: &str) -> Option<&TaskSummary> {
        self.current_tasks().iter().find(|t| t.id == id)
    }

    fn has_children_in_rows(&self, id: &str) -> bool {
        self.current_tasks().iter().any(|t| t.parent.as_deref() == Some(id))
    }

    pub fn status_rank(&self, status: &str) -> f64 {
        let lower = status.to_lowercase();
        if let Some(sid) = self.source.as_ref().and_then(|s| s.space_id()) {
            if let Some(sp) = self.spaces.iter().find(|s| s.id == sid) {
                if let Some(st) = sp.statuses.iter().find(|s| s.status.to_lowercase() == lower) {
                    return st.order();
                }
            }
        }
        for sp in &self.spaces {
            if let Some(st) = sp.statuses.iter().find(|s| s.status.to_lowercase() == lower) {
                return 100.0 + st.order();
            }
        }
        if text::is_done_status(&lower) {
            900.0
        } else if lower.contains("review") {
            520.0
        } else if lower.contains("progress") {
            510.0
        } else if lower.contains("block") {
            505.0
        } else {
            500.0
        }
    }

    pub fn status_color(&self, status: &str) -> Color {
        self.status_colors
            .get(&status.to_lowercase())
            .copied()
            .unwrap_or_else(|| text::fallback_status_color(status, None))
    }

     /// Rank used to order status groups: the API's per-list orderindex when
    /// known, else the space definition / keyword heuristics.
    pub fn task_rank(&self, t: &TaskSummary) -> f64 {
        let fine = t.status_order.unwrap_or_else(|| self.status_rank(&t.status));
        t.type_rank() * 10_000.0 + fine
    }

    pub fn rebuild_rows(&mut self) {
        let prev_id = self.table_state.selected().and_then(|i| self.rows.get(i)).map(|r| r.id.clone());
        let prev_idx = self.table_state.selected();
        let tasks = self.current_tasks().to_vec();
        let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        let filter = self.task_filter.to_lowercase();
        let filters = self.filters.clone();
        let assignee_data = tasks.iter().any(|t| t.has_assignee_data);
        // the API's "my tasks" includes done tasks; hide them like cup does unless asked
        let hide_done = matches!(self.source, Some(TaskSource::Assigned)) && !self.include_closed;
        let matches = |t: &TaskSummary| {
            !(hide_done && t.is_done())
                && filters.matches(t, assignee_data)
                && (filter.is_empty()
                    || text::fuzzy_score(&t.name, &filter).is_some()
                    || t.id.to_lowercase().contains(&filter)
                    || t.status.to_lowercase().contains(&filter))
        };

        let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut roots: Vec<usize> = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            match t.parent.as_deref() {
                Some(p) if ids.contains(p) && p != t.id => children.entry(p).or_default().push(i),
                _ => roots.push(i),
            }
        }
        let grouped = self.sort == SortMode::Status;
        let filtering = !filter.is_empty() || !filters.is_empty();
        let expanded_tasks = self.expanded_tasks.clone();
        let sort_key = |i: usize| -> (u64, u8, i64, String, usize) {
            let t = &tasks[i];
            match self.sort {
                // inside a status group: priority, then due, then name
                SortMode::Status => (0, text::priority_rank(&t.priority), t.due_ms().unwrap_or(i64::MAX), t.name.to_lowercase(), i),
                SortMode::Priority => (
                    text::priority_rank(&t.priority) as u64,
                    0,
                    (self.task_rank(t) * 1000.0) as i64,
                    t.name.to_lowercase(),
                    i,
                ),
                SortMode::Due => (
                    t.due_ms().map(|d| d as u64).unwrap_or(u64::MAX),
                    text::priority_rank(&t.priority),
                    0,
                    t.name.to_lowercase(),
                    i,
                ),
                SortMode::Name => (0, 0, 0, t.name.to_lowercase(), i),
                SortMode::Default => (0, 0, 0, String::new(), i),
            }
        };
        roots.sort_by_key(|&i| sort_key(i));
        for v in children.values_mut() {
            v.sort_by_key(|&i| sort_key(i));
        }

        struct Walk<'a> {
            tasks: &'a [TaskSummary],
            children: &'a HashMap<&'a str, Vec<usize>>,
            matches: &'a dyn Fn(&TaskSummary) -> bool,
            expanded: &'a HashSet<String>,
            filtering: bool,
        }
        fn walk(w: &Walk, i: usize, depth: u8, rows: &mut Vec<TaskRow>, force: bool) -> bool {
            let t = &w.tasks[i];
            let self_match = (w.matches)(t) || force;
            let mut sub = Vec::new();
            let mut any_child = false;
            let mut child_count = 0;
            if let Some(ch) = w.children.get(t.id.as_str()) {
                for &c in ch {
                    if walk(w, c, depth + 1, &mut sub, self_match) {
                        any_child = true;
                        child_count += 1;
                    }
                }
            }
            if self_match || any_child {
                // parents start collapsed; a filter that only hits a child opens the parent
                let collapsed = child_count > 0 && !w.expanded.contains(&t.id) && !(w.filtering && !(w.matches)(t));
                rows.push(TaskRow { id: t.id.clone(), depth, header: None, child_count, collapsed });
                if !collapsed {
                    rows.extend(sub);
                }
                return true;
            }
            false
        }
        let w = Walk { tasks: &tasks, children: &children, matches: &matches, expanded: &expanded_tasks, filtering };

        let mut rows = Vec::new();
        if grouped {
            // ClickUp style: a parent task is grouped by its own status and its
            // subtasks follow it, whatever their status
            // (display name, rank, top-level count, rows)
            let mut groups: Vec<(String, f64, usize, Vec<TaskRow>)> = Vec::new();
            for r in roots {
                let mut sub = Vec::new();
                if !walk(&w, r, 0, &mut sub, false) {
                    continue;
                }
                let t = &tasks[r];
                let key = t.status.trim().to_lowercase();
                let rank = self.task_rank(t);
                match groups.iter_mut().find(|g| g.0.trim().to_lowercase() == key) {
                    Some(g) => {
                        g.2 += 1;
                        g.3.extend(sub);
                        if rank < g.1 {
                            g.1 = rank;
                        }
                    }
                    None => groups.push((t.status.trim().to_string(), rank, 1, sub)),
                }
            }
            groups.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (name, _, count, sub) in groups {
                rows.push(TaskRow {
                    id: format!("#group:{}", name.to_lowercase()),
                    depth: 0,
                    header: Some((name, count)),
                    child_count: sub.len(),
                    collapsed: false,
                });
                rows.extend(sub);
            }
        } else {
            for r in roots {
                walk(&w, r, 0, &mut rows, false);
            }
        }
        self.rows = rows;

        // restore selection: same task if still visible, else nearest task row
        let target = self.pending_select.take().or(prev_id);
        let idx = target
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .or_else(|| {
                if self.rows.is_empty() {
                    None
                } else {
                    let start = prev_idx.unwrap_or(0).min(self.rows.len() - 1);
                    (start..self.rows.len())
                        .find(|&i| !self.rows[i].is_header())
                        .or_else(|| (0..start).rev().find(|&i| !self.rows[i].is_header()))
                }
            });
        self.table_state.select(idx);
        if idx.is_none() {
            *self.table_state.offset_mut() = 0;
        }
        self.sync_detail_to_selection();
    }

     pub fn selected_task_id(&self) -> Option<String> {
        self.table_state
            .selected()
            .and_then(|i| self.rows.get(i))
            .filter(|r| !r.is_header())
            .map(|r| r.id.clone())
    }

     pub fn tasks_move(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let mut i = self.table_state.selected().unwrap_or(0) as i32;
        let step = if delta >= 0 { 1 } else { -1 };
        let mut remaining = delta.abs();
        while remaining > 0 {
            let mut j = i + step;
            while j >= 0 && j < n && self.rows[j as usize].is_header() {
                j += step;
            }
            if j < 0 || j >= n {
                break;
            }
            i = j;
            remaining -= 1;
        }
        self.table_state.select(Some(i as usize));
        self.sync_detail_to_selection();
    }

    pub fn selected_row(&self) -> Option<&TaskRow> {
        self.table_state.selected().and_then(|i| self.rows.get(i))
    }

    /// Space on a parent task: show or hide its subtasks.
    pub fn toggle_fold(&mut self) {
        let Some(row) = self.selected_row().cloned() else { return };
        if row.is_header() || row.child_count == 0 {
            return;
        }
        if !self.expanded_tasks.remove(&row.id) {
            self.expanded_tasks.insert(row.id.clone());
        }
        self.rebuild_rows();
    }

    /// Left / h: collapse the current parent, or jump from a subtask to its parent.
    /// False = nothing to do here (caller falls back to the sidebar).
    pub fn fold_current(&mut self) -> bool {
        let Some(row) = self.selected_row().cloned() else { return false };
        if row.child_count > 0 && !row.collapsed {
            self.expanded_tasks.remove(&row.id);
            self.rebuild_rows();
            return true;
        }
        if row.depth > 0 {
            let sel = self.table_state.selected().unwrap_or(0);
            if let Some(p) = (0..sel).rev().find(|&i| self.rows[i].depth < row.depth && !self.rows[i].is_header()) {
                self.table_state.select(Some(p));
                self.sync_detail_to_selection();
            }
            return true;
        }
        false
    }

    /// Right / l: expand the current parent. False = nothing to expand.
    pub fn unfold_current(&mut self) -> bool {
        let Some(row) = self.selected_row().cloned() else { return false };
        if !row.collapsed || row.child_count == 0 {
            return false;
        }
        self.expanded_tasks.insert(row.id.clone());
        self.rebuild_rows();
        true
    }

    /// `-` / `=`: collapse or expand every parent task in this view.
    pub fn fold_all_tasks(&mut self, collapsed: bool) {
        if collapsed {
            self.expanded_tasks.clear();
        } else {
            let ids: Vec<String> = self.current_tasks().iter().filter_map(|t| t.parent.clone()).collect();
            self.expanded_tasks.extend(ids);
        }
        self.rebuild_rows();
    }

    pub fn tasks_jump(&mut self, to_end: bool) {
        let idx = if to_end {
            (0..self.rows.len()).rev().find(|&i| !self.rows[i].is_header())
        } else {
            (0..self.rows.len()).find(|&i| !self.rows[i].is_header())
        };
        if let Some(i) = idx {
            self.table_state.select(Some(i));
            self.sync_detail_to_selection();
        }
    }

    /// When the detail pane is open it follows the table highlight (debounced).
    pub fn sync_detail_to_selection(&mut self) {
        if !self.detail_open {
            return;
        }
        let Some(id) = self.selected_task_id() else { return };
        if self.detail_id.as_deref() != Some(id.as_str()) {
            self.detail_id = Some(id.clone());
            self.detail_reset_view();
            self.pending_detail = Some((id, Instant::now()));
        }
    }

    pub fn open_detail(&mut self) {
        let Some(id) = self.selected_task_id() else { return };
        self.detail_open = true;
        self.focus = Focus::Detail;
        if self.detail_id.as_deref() != Some(id.as_str()) {
            self.detail_reset_view();
        }
        self.detail_id = Some(id.clone());
        self.pending_detail = None;
        self.load_detail(&id, false);
    }

    fn detail_reset_view(&mut self) {
        self.detail_scroll = 0;
        self.detail_cursor = (0, 0);
        self.detail_want_col = 0;
        self.visual = None;
        self.pending_g = false;
    }

    pub fn close_detail(&mut self) {
        self.detail_open = false;
        self.visual = None;
        self.pending_detail = None;
        if self.focus == Focus::Detail {
            self.focus = Focus::Tasks;
        }
    }

    /// Task the action keys apply to.
    pub fn current_task_id(&self) -> Option<String> {
        match self.focus {
            Focus::Detail => self.detail_id.clone().or_else(|| self.selected_task_id()),
            _ => self.selected_task_id(),
        }
    }

    pub fn cycle_focus(&mut self, backwards: bool) {
        let mut order = Vec::new();
        if self.show_sidebar {
            order.push(Focus::Sidebar);
        }
        if !(self.narrow && self.detail_open) {
            order.push(Focus::Tasks);
        }
        if self.detail_open {
            order.push(Focus::Detail);
        }
        if order.is_empty() {
            return;
        }
        let cur = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = if backwards {
            (cur + order.len() - 1) % order.len()
        } else {
            (cur + 1) % order.len()
        };
        self.focus = order[next];
    }

    // ------------------------------------------------------------------
    // Actions (writes through cup)
    // ------------------------------------------------------------------

    fn run_action(&mut self, label: impl Into<String>, args: Vec<String>, task_id: Option<String>, created: bool) {
        let label = label.into();
        self.toast(format!("{label}…"), ToastKind::Info);
        let c = self.client.clone();
        let l = label.clone();
        self.spawn(async move {
            let result = c.run_write(&args).await;
            Msg::ActionDone { label: l, task_id, result, created }
        });
    }

     /// Statuses worth offering: the current space's definition first, then
    /// whatever the loaded tasks (and the task itself) are using.
    fn status_items(&self, task_id: Option<&str>) -> Vec<PickerItem> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut items: Vec<(f64, PickerItem)> = Vec::new();
        let mut push = |name: &str, color: Option<Color>, order: f64, seen: &mut HashSet<String>| {
            let name = name.trim();
            let key = name.to_lowercase();
            if name.is_empty() || !seen.insert(key) {
                return;
            }
            items.push((
                order,
                PickerItem { label: name.to_string(), value: name.to_string(), color, hint: String::new() },
            ));
        };
        if let Some(sid) = self.source.as_ref().and_then(|s| s.space_id()) {
            if let Some(sp) = self.spaces.iter().find(|s| s.id == sid) {
                for st in &sp.statuses {
                    push(&st.status, st.color.as_deref().and_then(text::hex_color), st.order(), &mut seen);
                }
            }
        }
        for t in self.current_tasks() {
            let c = self.status_color(&t.status);
            let r = self.task_rank(t);
            push(&t.status, Some(c), r, &mut seen);
        }
        if let Some(d) = task_id.and_then(|id| self.details.get(id)) {
            let name = d.task.status_name();
            let c = self.status_color(&name);
            push(&name, Some(c), 0.0, &mut seen);
        }
        items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        items.into_iter().map(|(_, i)| i).collect()
    }

    pub fn open_status_picker(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        let items = self.status_items(Some(&id));
        if items.is_empty() {
            self.toast("No statuses known for this list yet", ToastKind::Error);
            return;
        }
        let current = self.task_by_id(&id).map(|t| t.status.clone()).unwrap_or_default();
        let selected = items.iter().position(|i| i.value.eq_ignore_ascii_case(&current)).unwrap_or(0);
        self.mode = Mode::Picker(PickerState::new("Set status", items, selected, PickerPurpose::Status { task_id: id }));
    }

    pub fn open_priority_picker(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        let items = ["urgent", "high", "normal", "low"]
            .iter()
            .map(|p| PickerItem {
                label: p.to_string(),
                value: p.to_string(),
                color: Some(text::priority_color(p)),
                hint: String::new(),
            })
            .collect::<Vec<_>>();
        let current = self.task_by_id(&id).map(|t| t.priority.clone()).unwrap_or_default();
        let selected = items.iter().position(|i| i.value == current).unwrap_or(2);
        self.mode = Mode::Picker(PickerState::new("Set priority", items, selected, PickerPurpose::Priority { task_id: id }));
    }

    pub fn open_assignee_picker(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        if self.members.is_empty() {
            self.toast("Members not loaded yet", ToastKind::Error);
            let c = self.client.clone();
            self.spawn(async move { Msg::Members(c.members().await) });
            return;
        }
        let assigned: HashSet<String> = self
            .details
            .get(&id)
            .map(|d| d.task.assignees.iter().map(|u| u.id_string()).collect())
            .unwrap_or_default();
        let mut members = self.members.clone();
        members.sort_by_key(|m| m.display().to_lowercase());
        let items = members
            .iter()
            .map(|m| {
                let uid = m.id_string();
                let is = assigned.contains(&uid);
                PickerItem {
                    label: m.display(),
                    value: uid,
                    color: m.color.as_deref().and_then(text::hex_color),
                    hint: if is { "assigned · Enter removes".into() } else { m.email.clone().unwrap_or_default() },
                }
            })
            .collect();
        self.mode = Mode::Picker(PickerState::new("Toggle assignee", items, 0, PickerPurpose::Assignee { task_id: id }));
    }

    pub fn open_sort_picker(&mut self) {
        let items = SortMode::ALL
            .iter()
            .map(|m| PickerItem { label: m.label().to_string(), value: m.label().to_string(), color: None, hint: String::new() })
            .collect();
        let selected = SortMode::ALL.iter().position(|m| *m == self.sort).unwrap_or(0);
        self.mode = Mode::Picker(PickerState::new("Sort tasks by", items, selected, PickerPurpose::Sort));
    }

     // ---- structured filters -------------------------------------------

    pub fn open_filter_menu(&mut self) {
        let me = self.me.as_ref().map(|u| u.id_string());
        let f = &self.filters;
        let item = |label: &str, value: &str, hint: String| PickerItem { label: label.into(), value: value.into(), color: None, hint };
        let assignee_hint = if f.assignees.is_empty() {
            "anyone".to_string()
        } else {
            TaskFilters { assignees: f.assignees.clone(), ..Default::default() }.summary(me.as_deref())
        };
        let items = vec![
            item("Status", "status", if f.statuses.is_empty() { "any".into() } else { f.statuses.join(", ") }),
            item("Priority", "priority", if f.priorities.is_empty() { "any".into() } else { f.priorities.join(", ") }),
            item("Assignee", "assignee", assignee_hint),
            item("Clear all filters", "clear", String::new()),
        ];
        self.mode = Mode::Picker(PickerState::new("Filter tasks by", items, 0, PickerPurpose::FilterMenu));
    }

    fn open_multi_picker(&mut self, title: &str, items: Vec<PickerItem>, checked: &[String], purpose: PickerPurpose) {
        let mut p = PickerState::new(format!("{title}  ·  Tab toggles, Enter applies"), items, 0, purpose);
        p.multi = true;
        p.checked = checked.iter().map(|s| s.to_lowercase()).collect();
        self.mode = Mode::Picker(p);
    }

    pub fn open_filter_status(&mut self) {
        let items = self.status_items(None);
        if items.is_empty() {
            self.toast("No statuses known yet", ToastKind::Error);
            return;
        }
        let checked = self.filters.statuses.clone();
        self.open_multi_picker("Filter by status", items, &checked, PickerPurpose::FilterStatus);
    }

    pub fn open_filter_priority(&mut self) {
        let items = ["urgent", "high", "normal", "low", "none"]
            .iter()
            .map(|p| PickerItem { label: p.to_string(), value: p.to_string(), color: Some(text::priority_color(p)), hint: String::new() })
            .collect();
        let checked = self.filters.priorities.clone();
        self.open_multi_picker("Filter by priority", items, &checked, PickerPurpose::FilterPriority);
    }

    pub fn open_filter_assignee(&mut self) {
        let me = self.me.as_ref().map(|u| u.id_string());
        let mut items = Vec::new();
        if let Some(id) = &me {
            items.push(PickerItem { label: "Me".into(), value: id.clone(), color: Some(text::ACCENT), hint: String::new() });
        }
        items.push(PickerItem { label: "Unassigned".into(), value: "none".into(), color: Some(text::DIM), hint: "client-side".into() });
        let mut members = self.members.clone();
        members.sort_by_key(|m| m.display().to_lowercase());
        for m in members {
            let uid = m.id_string();
            if Some(&uid) == me.as_ref() {
                continue;
            }
            items.push(PickerItem { label: m.display(), value: uid, color: m.color.as_deref().and_then(text::hex_color), hint: m.email.clone().unwrap_or_default() });
        }
        let checked = self.filters.assignees.clone();
        self.open_multi_picker("Filter by assignee", items, &checked, PickerPurpose::FilterAssignee);
    }

    pub fn apply_filters(&mut self) {
        self.load_current_tasks(false);
        self.rebuild_rows();
        if self.filters.is_empty() {
            self.toast("Filters cleared", ToastKind::Info);
        }
    }

    /// Esc: clear structured filters. Returns true if there was something to clear.
    pub fn clear_filters(&mut self) -> bool {
        if self.filters.is_empty() {
            return false;
        }
        self.filters = TaskFilters::default();
        self.apply_filters();
        true
    }

    /// Tab in a multi-select picker.
    pub fn picker_toggle(&mut self) {
        let Mode::Picker(p) = &mut self.mode else { return };
        if !p.multi {
            return;
        }
        let filtered = p.filtered();
        if let Some(&idx) = filtered.get(p.selected) {
            let v = p.items[idx].value.to_lowercase();
            if !p.checked.remove(&v) {
                p.checked.insert(v);
            }
        }
    }

    pub fn picker_submit(&mut self) {
        // highlighted item (if any) and, for multi pickers, the checked set
        let (highlighted, chosen) = match &self.mode {
            Mode::Picker(p) => {
                let highlighted = p.filtered().get(p.selected).map(|&i| p.items[i].value.clone());
                let chosen: Vec<String> = if !p.multi {
                    Vec::new()
                } else if p.checked.is_empty() {
                    highlighted.iter().cloned().collect()
                } else {
                    p.items.iter().filter(|i| p.is_checked(i)).map(|i| i.value.clone()).collect()
                };
                (highlighted, chosen)
            }
            _ => return,
        };
        let Mode::Picker(p) = &self.mode else { return };
        if highlighted.is_none() && chosen.is_empty() {
            return; // nothing matches the typed filter; keep the picker open
        }
        let label = |value: &str| p.items.iter().find(|i| i.value == value).map(|i| i.label.clone()).unwrap_or_else(|| value.to_string());
        let purpose = p.purpose.clone();
        let value = highlighted.unwrap_or_default();
        let value_label = label(&value);
        self.mode = Mode::Normal;
        match purpose {
            PickerPurpose::Status { task_id } => {
                let args = vec!["update".into(), task_id.clone(), "-s".into(), value.clone()];
                self.run_action(format!("Status → {value_label}"), args, Some(task_id), false);
            }
            PickerPurpose::Priority { task_id } => {
                let args = vec!["update".into(), task_id.clone(), "--priority".into(), value.clone()];
                self.run_action(format!("Priority → {value_label}"), args, Some(task_id), false);
            }
            PickerPurpose::Assignee { task_id } => {
                let assigned = self.details.get(&task_id).map(|d| d.task.has_assignee(&value)).unwrap_or(false);
                let flag = if assigned { "--remove" } else { "--to" };
                let verb = if assigned { "Unassigned" } else { "Assigned" };
                let args = vec!["assign".into(), task_id.clone(), flag.into(), value.clone()];
                self.run_action(format!("{verb} {value_label}"), args, Some(task_id), false);
            }
            PickerPurpose::Sort => {
                if let Some(m) = SortMode::ALL.iter().find(|m| m.label() == value) {
                    self.sort = *m;
                    self.rebuild_rows();
                }
            }
            PickerPurpose::FilterMenu => match value.as_str() {
                "status" => self.open_filter_status(),
                "priority" => self.open_filter_priority(),
                "assignee" => self.open_filter_assignee(),
                _ => {
                    self.clear_filters();
                }
            },
            PickerPurpose::FilterStatus => {
                self.filters.statuses = chosen;
                self.apply_filters();
            }
            PickerPurpose::FilterPriority => {
                self.filters.priorities = chosen;
                self.apply_filters();
            }
            PickerPurpose::FilterAssignee => {
                if !chosen.is_empty() && !self.current_tasks().iter().any(|t| t.has_assignee_data) && !self.current_tasks().is_empty() {
                    self.toast("This view has no assignee data; open a list for assignee filtering", ToastKind::Error);
                }
                self.filters.assignees = chosen;
                self.apply_filters();
            }
        }
    }

    pub fn toggle_assign_me(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        let Some(me) = self.me.as_ref().map(|u| u.id_string()) else {
            self.toast("Current user unknown (cup auth failed?)", ToastKind::Error);
            return;
        };
        let assigned = self.details.get(&id).map(|d| d.task.has_assignee(&me)).unwrap_or(false);
        let (flag, label) = if assigned { ("--remove", "Unassigned me") } else { ("--to", "Assigned to me") };
        let args = vec!["assign".into(), id.clone(), flag.into(), "me".into()];
        self.run_action(label, args, Some(id), false);
    }

    pub fn open_input(&mut self, purpose: InputPurpose) {
        let (title, hint, initial): (&str, String, String) = match &purpose {
            InputPurpose::FilterTasks => ("Filter tasks", "fuzzy on name · Esc clears".into(), self.task_filter.clone()),
            InputPurpose::FilterSidebar => ("Filter sidebar", "spaces, folders, lists".into(), self.sidebar_filter.clone()),
            InputPurpose::Search { scope } => {
                let hint = match scope {
                    SearchScope::Space { name, .. } => format!("in space \"{name}\", all assignees · Ctrl-g searches the whole workspace"),
                    SearchScope::Mine => "my tasks, all spaces · open a list first to search its space · Ctrl-g: whole workspace".to_string(),
                    SearchScope::Workspace => "every list in the workspace, all assignees · slow, 1–2 minutes".to_string(),
                };
                ("Search tasks", hint, String::new())
            }
            InputPurpose::NewTask { parent: None, .. } => ("New task", "name · created in current list".into(), String::new()),
            InputPurpose::NewTask { parent: Some(_), .. } => ("New subtask", "name · under selected task".into(), String::new()),
            InputPurpose::Rename { task_id } => {
                let cur = self.task_by_id(task_id).map(|t| t.name.clone()).unwrap_or_default();
                ("Rename task", String::new(), cur)
            }
            InputPurpose::DueDate { .. } => ("Due date", "YYYY-MM-DD, YYYY-MM-DDTHH:MM, or 'none'".into(), String::new()),
            InputPurpose::Tags { .. } => ("Tags", "comma separated · prefix - to remove".into(), String::new()),
        };
        let cursor = initial.chars().count();
        self.mode = Mode::Input(InputState {
            title: title.into(),
            value: initial,
            cursor,
            purpose,
            hint,
        });
    }

    /// Default search scope: the space of the current list, else the space
    /// highlighted in the sidebar, else just my tasks.
    pub fn default_search_scope(&self) -> SearchScope {
        let sid = self
            .source
            .as_ref()
            .and_then(|s| s.space_id().map(str::to_string))
            .or_else(|| {
                self.sidebar_selected_row().and_then(|r| match r.kind {
                    RowKind::Space(id) => Some(id),
                    RowKind::Folder { space_id, .. } | RowKind::List { space_id, .. } => Some(space_id),
                    _ => None,
                })
            });
        match sid.and_then(|id| self.spaces.iter().find(|s| s.id == id)) {
            Some(sp) => SearchScope::Space { id: sp.id.clone(), name: sp.name.clone() },
            None => SearchScope::Mine,
        }
    }

    /// Called on every keystroke for live filters.
    pub fn input_changed(&mut self) {
        let Mode::Input(inp) = &self.mode else { return };
        match inp.purpose {
            InputPurpose::FilterTasks => {
                self.task_filter = inp.value.clone();
                self.rebuild_rows();
            }
            InputPurpose::FilterSidebar => {
                self.sidebar_filter = inp.value.clone();
                self.clamp_sidebar();
            }
            _ => {}
        }
    }

    pub fn input_cancel(&mut self) {
        let Mode::Input(inp) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
        match inp.purpose {
            InputPurpose::FilterTasks => {
                self.task_filter.clear();
                self.rebuild_rows();
            }
            InputPurpose::FilterSidebar => {
                self.sidebar_filter.clear();
                self.clamp_sidebar();
            }
            _ => {}
        }
    }

    pub fn input_submit(&mut self) {
        let Mode::Input(inp) = std::mem::replace(&mut self.mode, Mode::Normal) else { return };
        let value = inp.value.trim().to_string();
        match inp.purpose {
            InputPurpose::FilterTasks | InputPurpose::FilterSidebar => {}
            InputPurpose::Search { scope } => {
                if !value.is_empty() {
                    if scope == SearchScope::Workspace {
                        self.toast("Searching every list in the workspace, this takes a while…", ToastKind::Info);
                    }
                    self.set_source(TaskSource::Search { query: value, scope });
                    self.focus = Focus::Tasks;
                }
            }
            InputPurpose::NewTask { list_id, parent } => {
                if value.is_empty() {
                    return;
                }
                let mut args = vec!["create".into(), "-l".into(), list_id, "-n".into(), value.clone()];
                if let Some(p) = &parent {
                    args.push("--parent".into());
                    args.push(p.clone());
                }
                args.push("--json".into());
                self.run_action(format!("Created \"{}\"", text::truncate(&value, 30)), args, parent, true);
            }
            InputPurpose::Rename { task_id } => {
                if value.is_empty() {
                    return;
                }
                let args = vec!["update".into(), task_id.clone(), "-n".into(), value];
                self.run_action("Renamed", args, Some(task_id), false);
            }
            InputPurpose::DueDate { task_id } => {
                if value.is_empty() {
                    return;
                }
                let args = vec!["update".into(), task_id.clone(), "--due-date".into(), value.clone()];
                self.run_action(format!("Due → {value}"), args, Some(task_id), false);
            }
            InputPurpose::Tags { task_id } => {
                if value.is_empty() {
                    return;
                }
                let mut add = Vec::new();
                let mut remove = Vec::new();
                for t in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                    if let Some(r) = t.strip_prefix('-') {
                        remove.push(r.trim().to_string());
                    } else {
                        add.push(t.trim_start_matches('+').to_string());
                    }
                }
                let mut args = vec!["tag".into(), task_id.clone()];
                if !add.is_empty() {
                    args.push("--add".into());
                    args.push(add.join(","));
                }
                if !remove.is_empty() {
                    args.push("--remove".into());
                    args.push(remove.join(","));
                }
                self.run_action("Tags updated", args, Some(task_id), false);
            }
        }
    }

    pub fn start_new_task(&mut self, as_subtask: bool) {
        let parent = if as_subtask { self.current_task_id() } else { None };
        if as_subtask && parent.is_none() {
            self.toast("Select a task first", ToastKind::Error);
            return;
        }
        let list_id = match self.source.as_ref().and_then(|s| s.list_id()) {
            Some(l) => l.to_string(),
            None => {
                // parent decides the list; cup auto-detects it
                if as_subtask {
                    String::new()
                } else {
                    self.toast("Open a list first to create a task", ToastKind::Error);
                    return;
                }
            }
        };
        self.open_input(InputPurpose::NewTask { list_id, parent });
    }

    pub fn start_comment(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        self.effects.push(Effect::Editor { purpose: EditorPurpose::Comment { task_id: id }, initial: String::new() });
    }

    pub fn start_edit_description(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        let initial = self.details.get(&id).map(|d| d.task.body()).unwrap_or_default();
        if !self.details.contains_key(&id) {
            self.toast("Description not loaded yet, open the task first", ToastKind::Error);
            return;
        }
        self.effects.push(Effect::Editor { purpose: EditorPurpose::Description { task_id: id }, initial });
    }

    /// Result of the external editor. `path` is a temp file holding the text;
    /// it is handed to cup via `--*-file` and deleted afterwards.
    pub fn on_editor_result(&mut self, purpose: EditorPurpose, text: Option<String>, path: Option<std::path::PathBuf>) {
        let Some(t) = text else {
            self.toast("Editor closed without changes", ToastKind::Info);
            return;
        };
        if t.trim().is_empty() {
            if let Some(p) = &path {
                let _ = std::fs::remove_file(p);
            }
            self.toast("Empty, nothing sent", ToastKind::Info);
            return;
        }
        let Some(path) = path else { return };
        let p = path.to_string_lossy().to_string();
        let (label, args, task_id) = match purpose {
            EditorPurpose::Comment { task_id } => (
                "Comment posted",
                vec!["comment".to_string(), task_id.clone(), "--message-file".into(), p],
                task_id,
            ),
            EditorPurpose::Description { task_id } => (
                "Description updated",
                vec!["update".to_string(), task_id.clone(), "--description-file".into(), p],
                task_id,
            ),
        };
        self.toast(format!("{label}…"), ToastKind::Info);
        let c = self.client.clone();
        let l = label.to_string();
        self.spawn(async move {
            let result = c.run_write(&args).await;
            let _ = std::fs::remove_file(&path);
            Msg::ActionDone { label: l, task_id: Some(task_id), result, created: false }
        });
    }

    pub fn open_in_browser(&mut self) {
        let Some(id) = self.current_task_id() else { return };
        let url = self
            .task_by_id(&id)
            .map(|t| t.url.clone())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("https://app.clickup.com/t/{id}"));
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        match std::process::Command::new(opener)
            .arg(&url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.toast(format!("Opened {url}"), ToastKind::Success),
            Err(e) => self.toast(format!("open failed: {e}"), ToastKind::Error),
        }
    }

    /// Copy to the system clipboard via pbcopy / wl-copy / xclip.
    pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
        let candidates: &[(&str, &[&str])] = &[("pbcopy", &[]), ("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])];
        for (bin, args) in candidates {
            let child = std::process::Command::new(bin)
                .args(*args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if let Ok(mut ch) = child {
                use std::io::Write;
                if let Some(mut stdin) = ch.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = ch.wait();
                return Ok(());
            }
        }
        Err("No clipboard tool found (pbcopy/wl-copy/xclip)".into())
    }

    pub fn yank(&mut self, url: bool) {
        let Some(id) = self.current_task_id() else { return };
        let text = if url {
            self.task_by_id(&id)
                .map(|t| t.url.clone())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| format!("https://app.clickup.com/t/{id}"))
        } else {
            id.clone()
        };
        match Self::copy_to_clipboard(&text) {
            Ok(()) => self.toast(format!("Copied {text}"), ToastKind::Success),
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    // ---- vim-style cursor & visual mode in the detail pane -------------

    fn detail_row_len(&self, row: usize) -> usize {
        self.detail_rows_plain.get(row).map(|r| r.chars().count()).unwrap_or(0)
    }

    fn detail_last_row(&self) -> usize {
        self.detail_rows_plain.len().saturating_sub(1)
    }

    fn cursor_set_row(&mut self, row: usize) {
        let row = row.min(self.detail_last_row());
        let len = self.detail_row_len(row);
        let col = self.detail_want_col.min(len.saturating_sub(1));
        self.detail_cursor = (row, col);
        self.cursor_ensure_visible();
    }

    pub fn cursor_move_row(&mut self, delta: i32) {
        let row = (self.detail_cursor.0 as i32 + delta).clamp(0, self.detail_last_row() as i32) as usize;
        self.cursor_set_row(row);
    }

    pub fn cursor_move_col(&mut self, delta: i32) {
        let (row, col) = self.detail_cursor;
        let len = self.detail_row_len(row);
        let col = (col as i32 + delta).clamp(0, len.saturating_sub(1) as i32) as usize;
        self.detail_cursor = (row, col);
        self.detail_want_col = col;
    }

    pub fn cursor_line_start(&mut self) {
        self.detail_cursor.1 = 0;
        self.detail_want_col = 0;
    }

    pub fn cursor_line_end(&mut self) {
        let len = self.detail_row_len(self.detail_cursor.0);
        self.detail_cursor.1 = len.saturating_sub(1);
        self.detail_want_col = usize::MAX; // "stick to end of line", like `$`
    }

    /// `w` / `b`: next or previous word start, crossing rows when needed.
    pub fn cursor_word(&mut self, forward: bool) {
        let (mut row, col) = self.detail_cursor;
        let chars: Vec<char> = self.detail_rows_plain.get(row).map(|r| r.chars().collect()).unwrap_or_default();
        let is_word_start = |cs: &[char], i: usize| !cs[i].is_whitespace() && (i == 0 || cs[i - 1].is_whitespace());
        if forward {
            let mut i = col + 1;
            while i < chars.len() {
                if is_word_start(&chars, i) {
                    self.detail_cursor = (row, i);
                    self.detail_want_col = i;
                    return;
                }
                i += 1;
            }
            // next non-empty row
            while row < self.detail_last_row() {
                row += 1;
                let cs: Vec<char> = self.detail_rows_plain[row].chars().collect();
                if let Some(i) = (0..cs.len()).find(|&i| is_word_start(&cs, i)) {
                    self.detail_cursor = (row, i);
                    self.detail_want_col = i;
                    self.cursor_ensure_visible();
                    return;
                }
            }
        } else {
            let mut i = col;
            while i > 0 {
                i -= 1;
                if is_word_start(&chars, i) {
                    self.detail_cursor = (row, i);
                    self.detail_want_col = i;
                    return;
                }
            }
            while row > 0 {
                row -= 1;
                let cs: Vec<char> = self.detail_rows_plain[row].chars().collect();
                if let Some(i) = (0..cs.len()).rev().find(|&i| is_word_start(&cs, i)) {
                    self.detail_cursor = (row, i);
                    self.detail_want_col = i;
                    self.cursor_ensure_visible();
                    return;
                }
            }
        }
    }

    pub fn cursor_jump(&mut self, to_end: bool) {
        self.cursor_set_row(if to_end { self.detail_last_row() } else { 0 });
    }

    fn cursor_ensure_visible(&mut self) {
        let row = self.detail_cursor.0 as u16;
        let h = self.detail_view_height.max(1);
        if row < self.detail_scroll {
            self.detail_scroll = row;
        } else if row >= self.detail_scroll + h {
            self.detail_scroll = row + 1 - h;
        }
    }

    /// Mouse click inside the detail text: put the cursor there.
    pub fn detail_click(&mut self, x: u16, y: u16) {
        let inner = self.detail_inner;
        if !inner.contains(Position::new(x, y)) {
            return;
        }
        let row = self.detail_scroll as usize + (y - inner.y) as usize;
        if row > self.detail_last_row() {
            return;
        }
        let col = ((x - inner.x) as usize).min(self.detail_row_len(row).saturating_sub(1));
        self.detail_cursor = (row, col);
        self.detail_want_col = col;
    }

    /// `v` / `V`: start, switch or end a selection at the cursor.
    pub fn visual_toggle(&mut self, linewise: bool) {
        match self.visual {
            Some(v) if v.linewise == linewise => self.visual = None,
            Some(v) => self.visual = Some(Visual { anchor: v.anchor, linewise }),
            None => self.visual = Some(Visual { anchor: self.detail_cursor, linewise }),
        }
    }

    /// `o`: jump to the other end of the selection.
    pub fn visual_swap(&mut self) {
        if let Some(v) = self.visual {
            let cur = self.detail_cursor;
            self.detail_cursor = v.anchor;
            self.detail_want_col = v.anchor.1;
            self.visual = Some(Visual { anchor: cur, linewise: v.linewise });
            self.cursor_ensure_visible();
        }
    }

    /// Ordered selection bounds (inclusive) and whether it is line-wise.
    pub fn visual_range(&self) -> Option<((usize, usize), (usize, usize), bool)> {
        let v = self.visual?;
        let (a, c) = (v.anchor, self.detail_cursor);
        let (s, e) = if a <= c { (a, c) } else { (c, a) };
        Some((s, e, v.linewise))
    }

    /// Text covered by the selection; wrapped pieces of one line are re-joined.
    fn visual_text(&self) -> Option<String> {
        let (s, e, linewise) = self.visual_range()?;
        let mut out = String::new();
        for r in s.0..=e.0.min(self.detail_last_row()) {
            let row: Vec<char> = self.detail_rows_plain[r].chars().collect();
            let (c0, c1) = if linewise {
                (0, row.len())
            } else {
                (if r == s.0 { s.1 } else { 0 }, if r == e.0 { (e.1 + 1).min(row.len()) } else { row.len() })
            };
            if r > s.0 {
                let same_line = self.detail_row_line.get(r) == self.detail_row_line.get(r - 1);
                out.push_str(if same_line { " " } else { "\n" });
            }
            if c0 < c1 {
                out.extend(row[c0..c1].iter());
            }
        }
        Some(out)
    }

    /// `y` in visual mode: copy and leave visual mode.
    pub fn visual_yank(&mut self) {
        let Some(text) = self.visual_text() else { return };
        let Some((s, e, _)) = self.visual_range() else { return };
        self.visual = None;
        self.detail_cursor = s;
        self.detail_want_col = s.1;
        self.cursor_ensure_visible();
        let what = if s.0 == e.0 {
            format!("{} chars", text.chars().count())
        } else {
            format!("{} lines", e.0 - s.0 + 1)
        };
        match Self::copy_to_clipboard(&text) {
            Ok(()) => self.toast(format!("Copied {what}"), ToastKind::Success),
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    pub fn toggle_closed(&mut self) {
        if !self.source.as_ref().map(|s| s.supports_closed_toggle()).unwrap_or(false) {
            self.toast("This view already decides which tasks to show", ToastKind::Info);
            return;
        }
        self.include_closed = !self.include_closed;
        self.load_current_tasks(false);
        self.rebuild_rows();
    }

    pub fn refresh_focused(&mut self) {
        match self.focus {
            Focus::Sidebar => {
                if let Some(row) = self.sidebar_selected_row() {
                    match row.kind {
                        RowKind::Space(id) | RowKind::Folder { space_id: id, .. } | RowKind::List { space_id: id, .. } => {
                            self.load_space_children(&id, true);
                        }
                        _ => {}
                    }
                }
                self.load_spaces();
                self.toast("Refreshing sidebar…", ToastKind::Info);
            }
            Focus::Tasks => {
                self.reload_current_tasks();
                self.toast("Refreshing tasks…", ToastKind::Info);
            }
            Focus::Detail => {
                if let Some(id) = self.detail_id.clone() {
                    self.subtasks.remove(&id);
                    self.load_detail(&id, true);
                    self.toast("Refreshing task…", ToastKind::Info);
                }
            }
        }
    }

    /// Scroll the text (mouse wheel); the cursor is dragged along so it stays on screen.
    pub fn detail_scroll_by(&mut self, delta: i32) {
        let cur = self.detail_scroll as i32;
        self.detail_scroll = (cur + delta).clamp(0, self.detail_max_scroll as i32) as u16;
        let top = self.detail_scroll as usize;
        let bottom = top + self.detail_view_height.max(1) as usize - 1;
        let (row, _) = self.detail_cursor;
        if row < top {
            self.cursor_set_row(top);
        } else if row > bottom {
            self.cursor_set_row(bottom);
        }
    }

    /// Subtasks to show in the detail pane for `id`.
    pub fn detail_subtasks(&self, id: &str) -> Vec<TaskSummary> {
        let from_rows: Vec<TaskSummary> = self
            .current_tasks()
            .iter()
            .filter(|t| t.parent.as_deref() == Some(id))
            .cloned()
            .collect();
        if !from_rows.is_empty() {
            return from_rows;
        }
        self.subtasks.get(id).cloned().unwrap_or_default()
    }
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
