//! Rendering.

use std::collections::HashMap;

use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListItem, Padding, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, Wrap,
};
use ratatui::Frame;

use crate::app::*;
use crate::model::*;
use crate::text::{self, ACCENT, DIM, ERR, MUTED, OK, WARN};

const SEL_FOCUSED: Color = Color::Rgb(58, 64, 88);
const SEL_BLURRED: Color = Color::Rgb(40, 42, 54);

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.narrow = area.width < 110;
    let [top, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);

    draw_topbar(f, app, top);

    let sidebar_w = if app.show_sidebar { (area.width / 3).clamp(24, 36) } else { 0 };
    let [side, main] = Layout::horizontal([Constraint::Length(sidebar_w), Constraint::Min(0)]).areas(body);
    app.layout.sidebar = side;
    if app.show_sidebar {
        draw_sidebar(f, app, side);
    }

    if app.detail_open {
        if app.narrow {
            app.layout.tasks = Rect::default();
            app.layout.detail = main;
            if app.focus == Focus::Tasks {
                app.focus = Focus::Detail;
            }
            draw_detail(f, app, main);
        } else {
            let [t, d] =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);
            app.layout.tasks = t;
            app.layout.detail = d;
            draw_tasks(f, app, t);
            draw_detail(f, app, d);
        }
    } else {
        app.layout.tasks = main;
        app.layout.detail = Rect::default();
        draw_tasks(f, app, main);
    }

    draw_statusbar(f, app, bottom);

    match &app.mode {
        Mode::Normal => {}
        Mode::Help => draw_help(f, area),
        Mode::Input(inp) => draw_input(f, inp, area),
        Mode::Picker(p) => draw_picker(f, p, area),
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(Color::Rgb(70, 74, 90))
    }
}

fn title_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    }
}

fn pane_block(title: Line<'static>, focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(border_style(focused))
        .title(title)
}

fn draw_topbar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" cup-tui ", Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ];
    let crumbs = app.source.as_ref().map(|s| s.breadcrumb()).unwrap_or_default();
    if crumbs.is_empty() {
        spans.push(Span::styled("ClickUp", Style::default().fg(MUTED)));
    } else {
        for (i, c) in crumbs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" › ", Style::default().fg(DIM)));
            }
            let last = i == crumbs.len() - 1;
            spans.push(Span::styled(
                c.clone(),
                if last { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::default().fg(MUTED) },
            ));
        }
    }
    if let Some(id) = app.detail_id.as_ref().filter(|_| app.detail_open) {
        spans.push(Span::styled(" › ", Style::default().fg(DIM)));
        spans.push(Span::styled(id.clone(), Style::default().fg(WARN)));
    }

    let mut right = String::new();
    if let Some(p) = &app.client.profile {
        right.push_str(&format!("[{p}] "));
    }
    if let Some(me) = &app.me {
        right.push_str(&me.display());
    }
    right.push(' ');
    let rw = right.chars().count() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(spans)), l);
    f.render_widget(Paragraph::new(right).style(Style::default().fg(MUTED)), r);
}

fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let rows = app.sidebar_rows();
    let inner_w = area.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth as usize);
            let line = match &row.kind {
                RowKind::Header => Line::from(Span::styled(
                    format!("{indent}{}", row.label),
                    Style::default().fg(DIM).add_modifier(Modifier::BOLD),
                )),
                RowKind::Virtual(src) => {
                    let icon = match src {
                        TaskSource::Assigned => "◉",
                        TaskSource::Inbox => "✉",
                        TaskSource::Overdue => "⚠",
                        _ => "•",
                    };
                    let st = if row.active {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(format!("{indent}{icon} "), Style::default().fg(if row.active { ACCENT } else { MUTED })),
                        Span::styled(text::truncate(&row.label, inner_w.saturating_sub(indent.len() + 2)), st),
                    ])
                }
                RowKind::Space(_) | RowKind::Folder { .. } => {
                    let arrow = if row.loading {
                        SPINNER[app.spinner]
                    } else if row.expanded {
                        "▾"
                    } else {
                        "▸"
                    };
                    let is_space = matches!(row.kind, RowKind::Space(_));
                    let name_style = if is_space {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(220, 200, 150))
                    };
                    let mut spans = vec![
                        Span::styled(format!("{indent}{arrow} "), Style::default().fg(DIM)),
                        Span::styled(
                            text::truncate(&row.label, inner_w.saturating_sub(indent.len() + 6)),
                            name_style,
                        ),
                    ];
                    if let Some(c) = row.count {
                        spans.push(Span::styled(format!(" {c}"), Style::default().fg(DIM)));
                    }
                    Line::from(spans)
                }
                RowKind::List { .. } => {
                    let st = if row.active {
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(210, 214, 225))
                    };
                    let mut spans = vec![
                        Span::styled(format!("{indent}≡ "), Style::default().fg(if row.active { ACCENT } else { DIM })),
                        Span::styled(text::truncate(&row.label, inner_w.saturating_sub(indent.len() + 7)), st),
                    ];
                    if let Some(c) = row.count {
                        spans.push(Span::styled(format!(" {c}"), Style::default().fg(DIM)));
                    }
                    Line::from(spans)
                }
            };
            ListItem::new(line)
        })
        .collect();

    let mut title = vec![Span::styled(" Workspace ", title_style(focused))];
    if !app.sidebar_filter.is_empty() {
        title.push(Span::styled(format!("/{} ", app.sidebar_filter), Style::default().fg(WARN)));
    }
    let list = List::new(items)
        .block(pane_block(Line::from(title), focused))
        .highlight_style(
            Style::default()
                .bg(if focused { SEL_FOCUSED } else { SEL_BLURRED })
                .add_modifier(Modifier::BOLD),
        );
    f.render_stateful_widget(list, area, &mut app.sidebar_state);
}

fn draw_tasks(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Tasks;
    let key = app.current_key();
    let loading = app.tasks_loading_now();
    let error = key.as_ref().and_then(|k| app.tasks_error.get(k)).cloned();
    let me = app.me.as_ref().map(|u| u.id_string());
    let task_count = app.rows.iter().filter(|r| !r.is_header()).count();

    let mut title = vec![Span::styled(
        format!(" {} ", app.source.as_ref().map(|s| s.title()).unwrap_or_else(|| "Tasks".into())),
        title_style(focused),
    )];
    if app.source.is_some() {
        let more = if app.has_more_pages() || loading { "…" } else { "" };
        title.push(Span::styled(format!("{task_count}{more} "), Style::default().fg(DIM)));
    }
    if !app.task_filter.is_empty() {
        title.push(Span::styled(format!("/{} ", app.task_filter), Style::default().fg(WARN)));
    }
    if !app.filters.is_empty() {
        title.push(Span::styled(format!("⚑ {} ", app.filters.summary(me.as_deref())), Style::default().fg(WARN)));
    }
    if app.include_closed && app.source.as_ref().map(|s| s.supports_closed_toggle()).unwrap_or(false) {
        title.push(Span::styled("+closed ", Style::default().fg(DIM)));
    }
    if app.source.as_ref().map(|s| App::default_sort_for(s) != app.sort).unwrap_or(false) {
        title.push(Span::styled(format!("sort:{} ", app.sort.label()), Style::default().fg(DIM)));
    }
    if loading {
        title.push(Span::styled(format!("{} ", SPINNER[app.spinner]), Style::default().fg(ACCENT)));
    }
    let block = pane_block(Line::from(title), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.source.is_none() {
        let msg = vec![
            Line::from(""),
            Line::from(Span::styled("Pick a list in the sidebar and press Enter.", Style::default().fg(MUTED))),
            Line::from(""),
            Line::from(vec![
                Span::styled("g", Style::default().fg(ACCENT).bold()),
                Span::styled(" search   ", Style::default().fg(DIM)),
                Span::styled("?", Style::default().fg(ACCENT).bold()),
                Span::styled(" all keys", Style::default().fg(DIM)),
            ]),
        ];
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), inner);
        return;
    }
    if let Some(e) = error {
        f.render_widget(
            Paragraph::new(vec![Line::from(""), Line::from(Span::styled(e, Style::default().fg(ERR)))])
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    if app.rows.is_empty() {
        let text = if loading || app.has_more_pages() {
            "Loading…".to_string()
        } else if !app.task_filter.is_empty() || !app.filters.is_empty() {
            "No tasks match the filters  (Esc clears)".to_string()
        } else {
            "No open tasks here  (x shows closed)".to_string()
        };
        f.render_widget(
            Paragraph::new(vec![Line::from(""), Line::from(Span::styled(text, Style::default().fg(MUTED)))])
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let tasks = app.current_tasks();
    let by_id: HashMap<&str, &TaskSummary> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let show_list_col = !matches!(app.source, Some(TaskSource::List { .. }));
    let show_assignee = tasks.iter().any(|t| t.has_assignee_data) && !matches!(app.source, Some(TaskSource::Assigned));
    let wide = inner.width >= 80;
    let id_w: u16 = if wide { 10 } else { 0 };
    let list_w: u16 = if show_list_col { 14 } else { 0 };
    let asg_w: u16 = if show_assignee { 14 } else { 0 };
    let ncols: u16 = 4 + wide as u16 + show_list_col as u16 + show_assignee as u16;
    let name_w = inner
        .width
        .saturating_sub(id_w + 12 + 6 + 10 + list_w + asg_w + (ncols - 1))
        .max(8) as usize;

    let mut header_cells = vec![];
    if wide {
        header_cells.push("ID");
    }
    header_cells.extend(["Status", "Pri", "Name"]);
    if show_assignee {
        header_cells.push("Assignee");
    }
    if show_list_col {
        header_cells.push("List");
    }
    header_cells.push("Due");
    let header = Row::new(header_cells.iter().map(|h| Cell::from(*h)))
        .style(Style::default().fg(DIM).add_modifier(Modifier::BOLD))
        .bottom_margin(0);

    let blank_cells = ncols as usize;
    let any_parents = app.rows.iter().any(|r| r.child_count > 0 && !r.is_header());
    let rows: Vec<Row> = app
        .rows
        .iter()
        .enumerate()
        .map(|(ri, r)| {
            if let Some((status, count)) = &r.header {
                // group header: status name in the status column, count in the name column
                let color = app.status_color(status);
                let mut cells: Vec<Cell> = Vec::with_capacity(blank_cells);
                if wide {
                    cells.push(Cell::from(Span::styled("●", Style::default().fg(color))));
                }
                cells.push(Cell::from(Span::styled(
                    text::truncate(status, 12),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )));
                cells.push(Cell::from(""));
                cells.push(Cell::from(Span::styled(
                    format!("{count} {}", if *count == 1 { "task" } else { "tasks" }),
                    Style::default().fg(DIM),
                )));
                while cells.len() < blank_cells {
                    cells.push(Cell::from(""));
                }
                return Row::new(cells).top_margin(if ri == 0 { 0 } else { 1 });
            }
            let Some(t) = by_id.get(r.id.as_str()) else {
                return Row::new(vec![Cell::from(r.id.clone())]);
            };
            let done = t.is_done();
            let dim_all = if done { Style::default().fg(DIM) } else { Style::default() };
            let status_color = if done { OK } else { app.status_color(&t.status) };

            let mut name_spans = Vec::new();
            let mut used = 0usize;
            if r.depth > 0 {
                let s = format!("{}↳ ", "  ".repeat(r.depth as usize - 1));
                used += s.chars().count();
                name_spans.push(Span::styled(s, Style::default().fg(DIM)));
            }
            if r.child_count > 0 {
                let s = if r.collapsed { "▸ " } else { "▾ " };
                used += 2;
                name_spans.push(Span::styled(s, Style::default().fg(MUTED)));
            } else if any_parents && r.depth == 0 {
                used += 2;
                name_spans.push(Span::raw("  "));
            }
            // a subtask shown at top level means its parent is not in this view: say which
            let parent_hint = if r.depth == 0 {
                t.parent.as_deref().map(|p| {
                    let pname = by_id.get(p).map(|x| x.name.clone()).unwrap_or_else(|| p.to_string());
                    format!("  ↑ {}", text::truncate(&pname, 28))
                })
            } else {
                None
            };
            let hint_w = parent_hint.as_ref().map(|h| h.chars().count()).unwrap_or(0);
            if !t.task_type.is_empty() && t.task_type.to_lowercase() != "task" {
                let s = format!("{} ", t.task_type);
                used += s.chars().count();
                name_spans.push(Span::styled(
                    s,
                    Style::default().fg(Color::Rgb(200, 120, 220)).add_modifier(Modifier::ITALIC),
                ));
            }
            name_spans.push(Span::styled(
                text::truncate(&t.name, name_w.saturating_sub(used + hint_w)),
                if done { dim_all.add_modifier(Modifier::CROSSED_OUT) } else { Style::default().fg(Color::Rgb(225, 228, 235)) },
            ));
            if let Some(h) = parent_hint {
                name_spans.push(Span::styled(h, Style::default().fg(DIM)));
            }
            if r.collapsed && r.child_count > 0 {
                name_spans.push(Span::styled(format!("  +{}", r.child_count), Style::default().fg(WARN)));
            }

            let due = match t.due_ms() {
                Some(ms) => {
                    let s = text::fmt_ms_date(ms);
                    let overdue = !done && text::is_past_ms(ms);
                    Span::styled(s, Style::default().fg(if overdue { ERR } else { MUTED }))
                }
                None => Span::raw(""),
            };

            let mut cells = Vec::new();
            if wide {
                cells.push(Cell::from(Span::styled(t.id.clone(), Style::default().fg(DIM))));
            }
            cells.extend([
                Cell::from(Span::styled(text::truncate(&t.status, 12), Style::default().fg(status_color))),
                Cell::from(Span::styled(
                    text::priority_glyph(&t.priority),
                    Style::default().fg(text::priority_color(&t.priority)),
                )),
                Cell::from(Line::from(name_spans)),
            ]);
            if show_assignee {
                cells.push(Cell::from(assignee_line(t, me.as_deref(), asg_w as usize)));
            }
            if show_list_col {
                cells.push(Cell::from(Span::styled(text::truncate(&t.list, 14), Style::default().fg(DIM))));
            }
            cells.push(Cell::from(due));
            Row::new(cells).style(dim_all)
        })
        .collect();

    let mut widths = Vec::new();
    if wide {
        widths.push(Constraint::Length(id_w));
    }
    widths.extend([Constraint::Length(12), Constraint::Length(6), Constraint::Min(8)]);
    if show_assignee {
        widths.push(Constraint::Length(asg_w));
    }
    if show_list_col {
        widths.push(Constraint::Length(list_w));
    }
    widths.push(Constraint::Length(10));

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(if focused { SEL_FOCUSED } else { SEL_BLURRED }).add_modifier(Modifier::BOLD))
        .highlight_symbol("");
    f.render_stateful_widget(table, inner, &mut app.table_state);

    if app.rows.len() > inner.height.saturating_sub(1) as usize {
        let mut sb = ScrollbarState::new(app.rows.len()).position(app.table_state.selected().unwrap_or(0));
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Style::default().fg(DIM)).track_symbol(None),
            area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
            &mut sb,
        );
    }
}

/// Assignee cell: one name in full, several as initials, "me" highlighted.
fn assignee_line(t: &TaskSummary, me: Option<&str>, width: usize) -> Line<'static> {
    if t.assignees.is_empty() {
        return Line::from(Span::styled("—", Style::default().fg(DIM)));
    }
    let mut spans = Vec::new();
    if t.assignees.len() == 1 {
        let u = &t.assignees[0];
        let is_me = Some(u.id_string().as_str()) == me;
        let c = if is_me { ACCENT } else { u.color.as_deref().and_then(text::hex_color).unwrap_or(MUTED) };
        let name = if is_me { "me".to_string() } else { u.display() };
        spans.push(Span::styled(text::truncate(&name, width), Style::default().fg(c)));
        return Line::from(spans);
    }
    let mut used = 0;
    for (i, u) in t.assignees.iter().enumerate() {
        let is_me = Some(u.id_string().as_str()) == me;
        let label = if is_me {
            "me".to_string()
        } else {
            u.display().split_whitespace().filter_map(|w| w.chars().next()).take(3).collect::<String>().to_uppercase()
        };
        let piece = if i == 0 { label } else { format!(" {label}") };
        if used + piece.chars().count() > width {
            spans.push(Span::styled("…", Style::default().fg(DIM)));
            break;
        }
        used += piece.chars().count();
        let c = if is_me { ACCENT } else { u.color.as_deref().and_then(text::hex_color).unwrap_or(MUTED) };
        spans.push(Span::styled(piece, Style::default().fg(c)));
    }
    Line::from(spans)
}

fn meta(label: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{:<11}", label), Style::default().fg(DIM))];
    spans.extend(value);
    Line::from(spans)
}

fn section(title: &str, extra: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(title.to_string(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {extra}"), Style::default().fg(DIM)),
        ]),
    ]
}

fn draw_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Detail;
    let Some(id) = app.detail_id.clone() else {
        let block = pane_block(Line::from(Span::styled(" Task ", title_style(focused))), focused);
        f.render_widget(block, area);
        return;
    };
    let loading = app.detail_loading.contains(&id);
    let mut title = vec![Span::styled(" Task ", title_style(focused)), Span::styled(format!("{id} "), Style::default().fg(WARN))];
    if let Some((_, _, linewise)) = app.visual_range() {
        title.push(Span::styled(
            if linewise { " V-LINE " } else { " VISUAL " },
            Style::default().fg(Color::Black).bg(WARN).add_modifier(Modifier::BOLD),
        ));
        title.push(Span::raw(" "));
    }
    if loading {
        title.push(Span::styled(format!("{} ", SPINNER[app.spinner]), Style::default().fg(ACCENT)));
    }
    let block = pane_block(Line::from(title), focused).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(cached) = app.details.get(&id) else {
        let msg = if let Some(e) = app.detail_error.get(&id) {
            Span::styled(e.clone(), Style::default().fg(ERR))
        } else {
            Span::styled("Loading…", Style::default().fg(MUTED))
        };
        f.render_widget(Paragraph::new(vec![Line::from(""), Line::from(msg)]).alignment(Alignment::Center).wrap(Wrap { trim: true }), inner);
        return;
    };
    let t = &cached.task;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // title
    lines.push(Line::from(Span::styled(t.name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD))));
    let mut idline = vec![Span::styled(t.id.clone(), Style::default().fg(DIM))];
    if let Some(c) = &t.custom_id {
        idline.push(Span::styled(format!("  {c}"), Style::default().fg(WARN)));
    }
    if !t.url.is_empty() {
        idline.push(Span::styled(format!("  {}", t.url), Style::default().fg(DIM).add_modifier(Modifier::UNDERLINED)));
    }
    lines.push(Line::from(idline));
    lines.push(Line::from(""));

    // meta
    let status = t.status_name();
    let status_color = t
        .status
        .as_ref()
        .and_then(|s| s.color.as_deref())
        .and_then(text::hex_color)
        .unwrap_or_else(|| app.status_color(&status));
    let mut status_spans = vec![Span::styled(format!(" {status} "), Style::default().fg(Color::Black).bg(status_color).add_modifier(Modifier::BOLD))];
    if t.archived {
        status_spans.push(Span::styled("  archived", Style::default().fg(WARN)));
    }
    lines.push(meta("Status", status_spans));
    if let Some(p) = t.priority_name() {
        lines.push(meta("Priority", vec![Span::styled(p.clone(), Style::default().fg(text::priority_color(&p)).bold())]));
    }
    if !t.assignees.is_empty() {
        let mut spans = Vec::new();
        for (i, u) in t.assignees.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(", "));
            }
            let c = u.color.as_deref().and_then(text::hex_color).unwrap_or(MUTED);
            spans.push(Span::styled(u.display(), Style::default().fg(c)));
        }
        lines.push(meta("Assignees", spans));
    } else {
        lines.push(meta("Assignees", vec![Span::styled("unassigned", Style::default().fg(DIM))]));
    }
    let due = text::fmt_date(&t.due_date);
    if !due.is_empty() {
        let overdue = t.due_date.as_deref().and_then(|s| s.parse::<i64>().ok()).map(text::is_past_ms).unwrap_or(false)
            && !text::is_done_status(&status);
        lines.push(meta("Due", vec![Span::styled(due, Style::default().fg(if overdue { ERR } else { Color::White }))]));
    }
    let start = text::fmt_date(&t.start_date);
    if !start.is_empty() {
        lines.push(meta("Start", vec![Span::raw(start)]));
    }
    if !t.tags.is_empty() {
        let mut spans = Vec::new();
        for tag in &t.tags {
            let c = tag.tag_bg.as_deref().and_then(text::hex_color).unwrap_or(MUTED);
            spans.push(Span::styled(format!(" {} ", tag.name), Style::default().fg(Color::Black).bg(c)));
            spans.push(Span::raw(" "));
        }
        lines.push(meta("Tags", spans));
    }
    let est = text::fmt_duration_ms(&t.time_estimate);
    let spent = text::fmt_duration_ms(&t.time_spent);
    if !est.is_empty() || !spent.is_empty() {
        let mut spans = Vec::new();
        if !est.is_empty() {
            spans.push(Span::raw(format!("{est} estimated")));
        }
        if !spent.is_empty() {
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", Style::default().fg(DIM)));
            }
            spans.push(Span::raw(format!("{spent} tracked")));
        }
        lines.push(meta("Time", spans));
    }
    let mut loc = Vec::new();
    if let Some(s) = app.spaces.iter().find(|s| Some(&s.id) == t.space.as_ref().and_then(|r| r.id.as_ref())) {
        loc.push(s.name.clone());
    }
    if let Some(n) = t.folder.as_ref().and_then(|r| r.name.clone()).filter(|n| n != "hidden") {
        loc.push(n);
    }
    if let Some(n) = t.list.as_ref().and_then(|r| r.name.clone()) {
        loc.push(n);
    }
    if !loc.is_empty() {
        lines.push(meta("Location", vec![Span::styled(loc.join(" › "), Style::default().fg(MUTED))]));
    }
    if let Some(p) = &t.parent {
        let pname = app.task_by_id(p).map(|x| format!("  {}", text::truncate(&x.name, 50))).unwrap_or_default();
        lines.push(meta("Parent", vec![Span::styled(p.clone(), Style::default().fg(WARN)), Span::styled(pname, Style::default().fg(MUTED))]));
    }
    let created = text::fmt_date(&t.date_created);
    let updated = text::relative(&t.date_updated);
    let mut when = Vec::new();
    if !created.is_empty() {
        when.push(Span::styled(format!("created {created}"), Style::default().fg(MUTED)));
        if let Some(c) = &t.creator {
            when.push(Span::styled(format!(" by {}", c.display()), Style::default().fg(DIM)));
        }
    }
    if !updated.is_empty() {
        if !when.is_empty() {
            when.push(Span::styled(" · ", Style::default().fg(DIM)));
        }
        when.push(Span::styled(format!("updated {updated}"), Style::default().fg(MUTED)));
    }
    if !when.is_empty() {
        lines.push(meta("", when));
    }
    for cf in &t.custom_fields {
        if let Some((name, value)) = custom_field_display(cf) {
            lines.push(meta(&text::truncate(&name, 10), vec![Span::raw(value)]));
        }
    }

    // description
    lines.extend(section("Description", ""));
    let body = t.body();
    if body.trim().is_empty() {
        lines.push(Line::from(Span::styled("(no description)", Style::default().fg(DIM))));
    } else {
        lines.extend(text::markdown_lines(&body));
    }

    // checklists
    for cl in &t.checklists {
        let name = cl.get("name").and_then(|n| n.as_str()).unwrap_or("Checklist");
        let items = cl.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
        let resolved = items.iter().filter(|i| i.get("resolved").and_then(|r| r.as_bool()).unwrap_or(false)).count();
        lines.extend(section(name, &format!("{resolved}/{}", items.len())));
        for it in items {
            let done = it.get("resolved").and_then(|r| r.as_bool()).unwrap_or(false);
            let n = it.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            lines.push(Line::from(vec![
                Span::styled(if done { "☑ " } else { "☐ " }, Style::default().fg(if done { OK } else { MUTED })),
                Span::styled(n, if done { Style::default().fg(DIM).add_modifier(Modifier::CROSSED_OUT) } else { Style::default() }),
            ]));
        }
    }

    // attachments
    if !t.attachments.is_empty() {
        lines.extend(section("Attachments", &t.attachments.len().to_string()));
        for a in &t.attachments {
            let title = a.get("title").and_then(|x| x.as_str()).unwrap_or("file");
            lines.push(Line::from(vec![Span::styled("⎘ ", Style::default().fg(DIM)), Span::raw(title.to_string())]));
        }
    }

    // subtasks
    let subs = app.detail_subtasks(&id);
    if !subs.is_empty() {
        let done = subs.iter().filter(|s| text::is_done_status(&s.status)).count();
        lines.extend(section("Subtasks", &format!("{done}/{}", subs.len())));
        for s in &subs {
            let sdone = text::is_done_status(&s.status);
            lines.push(Line::from(vec![
                Span::styled(format!("{:<12} ", text::truncate(&s.status, 12)), Style::default().fg(if sdone { OK } else { app.status_color(&s.status) })),
                Span::styled(s.name.clone(), if sdone { Style::default().fg(DIM) } else { Style::default() }),
                Span::styled(format!("  {}", s.id), Style::default().fg(DIM)),
            ]));
        }
    }

    // comments
    lines.extend(section("Comments", &cached.comments.len().to_string()));
    if cached.comments.is_empty() {
        lines.push(Line::from(Span::styled("(none yet — press c to write one)", Style::default().fg(DIM))));
    }
    for c in &cached.comments {
        lines.push(Line::from(vec![
            Span::styled(c.author(), Style::default().fg(Color::Rgb(220, 200, 150)).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", text::fmt_datetime(&c.date)), Style::default().fg(DIM)),
            Span::styled(format!("  {}", text::relative(&c.date)), Style::default().fg(DIM)),
        ]));
        lines.extend(text::markdown_lines(&c.text));
        lines.push(Line::from(""));
    }

    // wrap ourselves so the cursor and scrolling work in exact visual rows
    let w = inner.width.max(1) as usize;
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut row_line: Vec<usize> = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        let wrapped = wrap_line(l, w);
        row_line.extend(std::iter::repeat(li).take(wrapped.len()));
        rows.extend(wrapped);
    }
    let total = rows.len();
    let max_scroll = total.saturating_sub(inner.height as usize) as u16;
    app.detail_max_scroll = max_scroll;
    app.detail_view_height = inner.height;
    app.detail_inner = inner;
    app.detail_rows_plain = rows
        .iter()
        .map(|l| l.spans.iter().map(|sp| sp.content.as_ref()).collect::<String>())
        .collect();
    app.detail_row_line = row_line;
    // clamp the cursor to the new geometry
    let last = total.saturating_sub(1);
    let (mut cr, mut cc) = app.detail_cursor;
    cr = cr.min(last);
    let len = app.detail_rows_plain.get(cr).map(|r| r.chars().count()).unwrap_or(0);
    cc = cc.min(len.saturating_sub(1));
    app.detail_cursor = (cr, cc);
    if app.detail_scroll > max_scroll {
        app.detail_scroll = max_scroll;
    }

    // selection highlight
    if let Some((sel_s, sel_e, linewise)) = app.visual_range() {
        let sel_style = Style::default().bg(Color::Rgb(72, 84, 128));
        for r in sel_s.0..=sel_e.0.min(last) {
            let (c0, c1) = if linewise {
                (0, usize::MAX)
            } else {
                (if r == sel_s.0 { sel_s.1 } else { 0 }, if r == sel_e.0 { sel_e.1 } else { usize::MAX })
            };
            rows[r] = style_cols(&rows[r], c0, c1, sel_style, false);
        }
    }
    // cursor (only while the pane has focus)
    if focused && total > 0 {
        let cursor_style = Style::default().bg(Color::Rgb(215, 220, 235)).fg(Color::Black);
        rows[cr] = style_cols(&rows[cr], cc, cc, cursor_style, true);
    }

    let para = Paragraph::new(rows).scroll((app.detail_scroll, 0));
    f.render_widget(para, inner);

    if max_scroll > 0 {
        let mut sb = ScrollbarState::new(max_scroll as usize + 1).position(app.detail_scroll as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight).thumb_style(Style::default().fg(DIM)).track_symbol(None),
            area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }),
            &mut sb,
        );
    }
}

/// Re-style the characters in columns `from..=to` of a row (char indices).
/// With `cursor` set, a column past the end of the row is drawn as a blank cell.
fn style_cols(line: &Line<'static>, from: usize, to: usize, style: Style, cursor: bool) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for sp in &line.spans {
        let text: &str = sp.content.as_ref();
        let mut plain = String::new();
        let mut hit = String::new();
        let mut plain_after = String::new();
        for c in text.chars() {
            if col < from {
                plain.push(c);
            } else if col <= to {
                hit.push(c);
            } else {
                plain_after.push(c);
            }
            col += 1;
        }
        if !plain.is_empty() {
            spans.push(Span::styled(plain, sp.style));
        }
        if !hit.is_empty() {
            spans.push(Span::styled(hit, sp.style.patch(style)));
        }
        if !plain_after.is_empty() {
            spans.push(Span::styled(plain_after, sp.style));
        }
    }
    if cursor && from >= col {
        spans.push(Span::styled(" ", style));
    }
    Line::from(spans).style(line.style)
}

/// Word-wrap one styled line into rows of at most `width` cells, keeping span
/// styles and the line's own style (used for the visual-mode highlight).
fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;
    let width = width.max(1);
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cur_w = 0usize;
    let flush = |cur: &mut Vec<Span<'static>>, cur_w: &mut usize, rows: &mut Vec<Line<'static>>| {
        rows.push(Line::from(std::mem::take(cur)).style(line.style));
        *cur_w = 0;
    };
    for sp in &line.spans {
        // tokens: runs of whitespace or runs of non-whitespace
        let text: &str = sp.content.as_ref();
        let mut tokens: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut buf_ws: Option<bool> = None;
        for c in text.chars() {
            let ws = c.is_whitespace();
            if buf_ws != Some(ws) && !buf.is_empty() {
                tokens.push(std::mem::take(&mut buf));
            }
            buf_ws = Some(ws);
            buf.push(c);
        }
        if !buf.is_empty() {
            tokens.push(buf);
        }
        for tok in tokens {
            let tw = tok.width();
            let is_ws = tok.chars().all(char::is_whitespace);
            if cur_w + tw <= width {
                cur.push(Span::styled(tok, sp.style));
                cur_w += tw;
                continue;
            }
            if is_ws {
                // whitespace at a break point is dropped
                if cur_w > 0 {
                    flush(&mut cur, &mut cur_w, &mut rows);
                }
                continue;
            }
            if cur_w > 0 {
                flush(&mut cur, &mut cur_w, &mut rows);
            }
            if tw <= width {
                cur.push(Span::styled(tok, sp.style));
                cur_w = tw;
            } else {
                // a single word longer than the row: hard split
                let mut piece = String::new();
                let mut pw = 0usize;
                for c in tok.chars() {
                    let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    if pw + cw > width && pw > 0 {
                        cur.push(Span::styled(std::mem::take(&mut piece), sp.style));
                        flush(&mut cur, &mut cur_w, &mut rows);
                        pw = 0;
                    }
                    piece.push(c);
                    pw += cw;
                }
                cur.push(Span::styled(piece, sp.style));
                cur_w = pw;
            }
        }
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(Line::from(cur).style(line.style));
    }
    rows
}

fn key_hint(k: &str, d: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(k.to_string(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {d}  "), Style::default().fg(DIM)),
    ]
}

fn draw_statusbar(f: &mut Frame, app: &App, area: Rect) {
    let left: Line = if let Some(t) = &app.toast {
        let (fg, icon) = match t.kind {
            ToastKind::Info => (MUTED, "…"),
            ToastKind::Success => (OK, "✓"),
            ToastKind::Error => (ERR, "✗"),
        };
        Line::from(vec![Span::styled(format!(" {icon} {}", t.text), Style::default().fg(fg))])
    } else {
        let mut spans = vec![Span::raw(" ")];
        let hints: &[(&str, &str)] = match (&app.mode, app.focus) {
            (Mode::Normal, Focus::Sidebar) => &[("↑↓", "move"), ("⏎", "open"), ("←→", "fold"), ("/", "filter"), ("g", "search"), ("Tab", "pane"), ("?", "help")],
            (Mode::Normal, Focus::Tasks) => &[("⏎", "detail"), ("␣", "subtasks"), ("s", "status"), ("p", "prio"), ("a", "assign"), ("c", "comment"), ("n", "new"), ("/", "find"), ("f", "filter"), ("?", "help")],
            (Mode::Normal, Focus::Detail) if app.visual.is_some() => &[("hjkl", "extend"), ("w b 0 $", "word/line"), ("o", "swap end"), ("y", "copy"), ("Esc", "cancel")],
            (Mode::Normal, Focus::Detail) => &[("hjkl", "cursor"), ("v V", "select"), ("[ ]", "prev/next"), ("s", "status"), ("c", "comment"), ("E", "edit desc"), ("y", "copy URL"), ("Esc", "close")],
            _ => &[("Esc", "cancel"), ("⏎", "confirm")],
        };
        for (k, d) in hints {
            spans.extend(key_hint(k, d));
        }
        Line::from(spans)
    };

    let mut right = Vec::new();
    if app.inflight > 0 {
        right.push(Span::styled(format!("{} {} ", SPINNER[app.spinner], app.inflight), Style::default().fg(ACCENT)));
    }
    right.push(Span::styled("q quit ", Style::default().fg(DIM)));
    let right_line = Line::from(right);
    let rw = right_line.width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right_line).alignment(Alignment::Right), r);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn popup_block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(format!(" {title} "), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .padding(Padding::horizontal(1))
}

fn draw_input(f: &mut Frame, inp: &InputState, area: Rect) {
    let rect = centered(area, 72, 4);
    f.render_widget(Clear, rect);
    let block = popup_block(&inp.title);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [line, hint] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    // horizontal scroll so the cursor stays visible
    let width = inner.width.max(1) as usize;
    let chars: Vec<char> = inp.value.chars().collect();
    let start = inp.cursor.saturating_sub(width.saturating_sub(1));
    let visible: String = chars[start..].iter().take(width).collect();
    f.render_widget(Paragraph::new(visible), line);
    f.render_widget(Paragraph::new(Span::styled(inp.hint.clone(), Style::default().fg(DIM))), hint);
    f.set_cursor_position(Position::new(line.x + (inp.cursor - start) as u16, line.y));
}

fn draw_picker(f: &mut Frame, p: &PickerState, area: Rect) {
    let filtered = p.filtered();
    let h = (filtered.len() as u16 + 4).clamp(6, area.height.saturating_sub(4));
    let rect = centered(area, 56, h);
    f.render_widget(Clear, rect);
    let block = popup_block(&p.title);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [filter_area, sep, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(ACCENT)),
            Span::raw(p.filter.clone()),
            Span::styled("▏", Style::default().fg(MUTED)),
        ])),
        filter_area,
    );
    f.render_widget(Paragraph::new(Span::styled("─".repeat(inner.width as usize), Style::default().fg(Color::Rgb(60, 64, 80)))), sep);

    let visible = list_area.height as usize;
    let offset = if p.selected >= visible { p.selected + 1 - visible } else { 0 };
    let mut lines = Vec::new();
    for (vi, &idx) in filtered.iter().enumerate().skip(offset).take(visible) {
        let it = &p.items[idx];
        let selected = vi == p.selected;
        let marker = if selected { "▶ " } else { "  " };
        let mut spans = vec![Span::styled(marker, Style::default().fg(ACCENT))];
        if p.multi {
            let checked = p.is_checked(it);
            spans.push(Span::styled(
                if checked { "☑ " } else { "☐ " },
                Style::default().fg(if checked { OK } else { DIM }),
            ));
        }
        if let Some(c) = it.color {
            spans.push(Span::styled("● ", Style::default().fg(c)));
        }
        spans.push(Span::styled(
            it.label.clone(),
            if selected { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::default() },
        ));
        if !it.hint.is_empty() {
            spans.push(Span::styled(format!("  {}", it.hint), Style::default().fg(DIM)));
        }
        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::default().bg(SEL_FOCUSED));
        }
        lines.push(line);
    }
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled("  no matches", Style::default().fg(DIM))));
    }
    f.render_widget(Paragraph::new(lines), list_area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rows: &[(&str, &str)] = &[
        ("Navigation", ""),
        ("Tab / Shift-Tab, 1 2 3", "cycle / jump between panes"),
        ("j k ↑ ↓, PgUp PgDn, Home End", "move · Ctrl-d / Ctrl-u half page"),
        ("Enter", "sidebar: open list or fold · tasks: open detail"),
        ("h l ← →", "sidebar: fold / unfold · tasks: collapse / expand subtasks, then sidebar / detail"),
        ("Space", "show / hide the subtasks of the parent under the cursor (collapsed by default)"),
        ("- / =", "collapse / expand all parent tasks"),
        ("Esc", "clear text filter → clear filters → close detail"),
        ("[ ]", "previous / next task while in detail (also J / K)"),
        ("b", "toggle sidebar"),
        ("", ""),
        ("Finding things", ""),
        ("/", "live filter in the focused pane"),
        ("g", "search tasks in the current space (or my tasks)"),
        ("Ctrl-g", "search the whole workspace (slow: walks every list)"),
        ("f", "filter by status / priority / assignee (Tab toggles, Enter applies)"),
        ("x", "include closed tasks (lists & search)"),
        ("S", "sort: status, priority, due, name, clickup order"),
        ("r / R", "refresh focused pane / refresh everything"),
        ("", ""),
        ("Task actions", ""),
        ("s  p  a  m", "status · priority · assignee picker · assign/unassign me"),
        ("c  E", "comment · edit description  (opens $EDITOR)"),
        ("e  d  t", "rename · due date · tags (\"a, b, -old\")"),
        ("n  N", "new task in this list · new subtask of selected"),
        ("o  y  Y", "open in browser · copy URL · copy ID"),
        ("detail: h j k l", "move the cursor · w b words · 0 $ line · gg G top/bottom · Ctrl-d/u pages"),
        ("detail: v  V", "select by character / by line, then y copies, o swaps ends, Esc cancels"),
        ("", ""),
        ("q / Ctrl-c", "quit"),
    ];
    let w = 96u16;
    let h = rows.len() as u16 + 2;
    let rect = centered(area, w, h);
    f.render_widget(Clear, rect);
    let block = popup_block("Keys");
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            if d.is_empty() && !k.is_empty() {
                Line::from(Span::styled(k.to_string(), Style::default().fg(WARN).add_modifier(Modifier::BOLD)))
            } else {
                Line::from(vec![
                    Span::styled(format!("{:<30}", k), Style::default().fg(ACCENT)),
                    Span::styled(d.to_string(), Style::default().fg(MUTED)),
                ])
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}
