//! Keyboard and mouse handling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::app::*;
use crate::model::SearchScope;

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }
    if matches!(app.mode, Mode::Help) {
        app.mode = Mode::Normal;
        return;
    }
    if matches!(app.mode, Mode::Input(_)) {
        return handle_input_key(app, key);
    }
    if matches!(app.mode, Mode::Picker(_)) {
        return handle_picker_key(app, key);
    }
    handle_normal_key(app, key);
}

enum InputAct {
    None,
    Changed,
    Cancel,
    Submit,
}

fn handle_input_key(app: &mut App, key: KeyEvent) {
    let act = {
        let Mode::Input(inp) = &mut app.mode else { return };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let len = inp.value.chars().count();
        let byte_at = |s: &str, ci: usize| s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len());
        match key.code {
            KeyCode::Esc => InputAct::Cancel,
            KeyCode::Enter => InputAct::Submit,
            KeyCode::Left if ctrl || alt => {
                // word left
                let chars: Vec<char> = inp.value.chars().collect();
                let mut i = inp.cursor;
                while i > 0 && chars[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && chars[i - 1] != ' ' {
                    i -= 1;
                }
                inp.cursor = i;
                InputAct::None
            }
            KeyCode::Right if ctrl || alt => {
                let chars: Vec<char> = inp.value.chars().collect();
                let mut i = inp.cursor;
                while i < len && chars[i] != ' ' {
                    i += 1;
                }
                while i < len && chars[i] == ' ' {
                    i += 1;
                }
                inp.cursor = i;
                InputAct::None
            }
            KeyCode::Left => {
                inp.cursor = inp.cursor.saturating_sub(1);
                InputAct::None
            }
            KeyCode::Right => {
                inp.cursor = (inp.cursor + 1).min(len);
                InputAct::None
            }
            KeyCode::Home => {
                inp.cursor = 0;
                InputAct::None
            }
            KeyCode::End => {
                inp.cursor = len;
                InputAct::None
            }
            KeyCode::Char('a') if ctrl => {
                inp.cursor = 0;
                InputAct::None
            }
            KeyCode::Char('e') if ctrl => {
                inp.cursor = len;
                InputAct::None
            }
            KeyCode::Char('u') if ctrl => {
                let b = byte_at(&inp.value, inp.cursor);
                inp.value = inp.value[b..].to_string();
                inp.cursor = 0;
                InputAct::Changed
            }
            KeyCode::Char('k') if ctrl => {
                let b = byte_at(&inp.value, inp.cursor);
                inp.value.truncate(b);
                InputAct::Changed
            }
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = inp.value.chars().collect();
                let mut i = inp.cursor;
                while i > 0 && chars[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && chars[i - 1] != ' ' {
                    i -= 1;
                }
                let start = byte_at(&inp.value, i);
                let end = byte_at(&inp.value, inp.cursor);
                inp.value.replace_range(start..end, "");
                inp.cursor = i;
                InputAct::Changed
            }
            KeyCode::Backspace => {
                if inp.cursor > 0 {
                    let start = byte_at(&inp.value, inp.cursor - 1);
                    let end = byte_at(&inp.value, inp.cursor);
                    inp.value.replace_range(start..end, "");
                    inp.cursor -= 1;
                    InputAct::Changed
                } else {
                    InputAct::None
                }
            }
            KeyCode::Delete => {
                if inp.cursor < len {
                    let start = byte_at(&inp.value, inp.cursor);
                    let end = byte_at(&inp.value, inp.cursor + 1);
                    inp.value.replace_range(start..end, "");
                    InputAct::Changed
                } else {
                    InputAct::None
                }
            }
            KeyCode::Char(c) if !ctrl => {
                let b = byte_at(&inp.value, inp.cursor);
                inp.value.insert(b, c);
                inp.cursor += 1;
                InputAct::Changed
            }
            _ => InputAct::None,
        }
    };
    match act {
        InputAct::None => {}
        InputAct::Changed => app.input_changed(),
        InputAct::Cancel => app.input_cancel(),
        InputAct::Submit => app.input_submit(),
    }
}

fn handle_picker_key(app: &mut App, key: KeyEvent) {
    let submit = {
        let Mode::Picker(p) = &mut app.mode else { return };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let n = p.filtered().len();
        match key.code {
            KeyCode::Esc => {
                app.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => true,
            KeyCode::Up | KeyCode::BackTab => {
                p.selected = p.selected.saturating_sub(1);
                false
            }
            KeyCode::Tab if p.multi => {
                app.picker_toggle();
                return;
            }
            KeyCode::Down | KeyCode::Tab => {
                if n > 0 {
                    p.selected = (p.selected + 1).min(n - 1);
                }
                false
            }
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => {
                p.selected = p.selected.saturating_sub(1);
                false
            }
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => {
                if n > 0 {
                    p.selected = (p.selected + 1).min(n - 1);
                }
                false
            }
            KeyCode::Home => {
                p.selected = 0;
                false
            }
            KeyCode::End => {
                p.selected = n.saturating_sub(1);
                false
            }
            KeyCode::Backspace => {
                p.filter.pop();
                p.selected = 0;
                false
            }
            KeyCode::Char('u') if ctrl => {
                p.filter.clear();
                p.selected = 0;
                false
            }
            KeyCode::Char(c) if !ctrl => {
                p.filter.push(c);
                p.selected = 0;
                false
            }
            _ => false,
        }
    };
    if submit {
        app.picker_submit();
    }
}

fn handle_normal_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // in the detail pane `g` is the first half of `gg` and `b` is "word back"
    if app.focus == Focus::Detail && !ctrl {
        let was_pending = app.pending_g;
        app.pending_g = false;
        match key.code {
            KeyCode::Char('g') => {
                if was_pending {
                    app.cursor_jump(false);
                } else {
                    app.pending_g = true;
                }
                return;
            }
            KeyCode::Char('b') => {
                app.cursor_word(false);
                return;
            }
            _ => {}
        }
    } else {
        app.pending_g = false;
    }
    // global
    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return;
        }
        KeyCode::Char('?') => {
            app.mode = Mode::Help;
            return;
        }
        KeyCode::Tab => {
            app.cycle_focus(false);
            return;
        }
        KeyCode::BackTab => {
            app.cycle_focus(true);
            return;
        }
        KeyCode::Char('1') => {
            if app.show_sidebar {
                app.focus = Focus::Sidebar;
            }
            return;
        }
        KeyCode::Char('2') => {
            if !(app.narrow && app.detail_open) {
                app.focus = Focus::Tasks;
            }
            return;
        }
        KeyCode::Char('3') => {
            if app.detail_open {
                app.focus = Focus::Detail;
            }
            return;
        }
        KeyCode::Char('b') => {
            app.show_sidebar = !app.show_sidebar;
            if !app.show_sidebar && app.focus == Focus::Sidebar {
                app.focus = Focus::Tasks;
            }
            return;
        }
        KeyCode::Char('r') => {
            app.refresh_focused();
            return;
        }
        KeyCode::Char('R') => {
            app.hard_refresh();
            return;
        }
        KeyCode::Char('g') if ctrl => {
            app.open_input(InputPurpose::Search { scope: SearchScope::Workspace });
            return;
        }
        KeyCode::Char('g') => {
            let scope = app.default_search_scope();
            app.open_input(InputPurpose::Search { scope });
            return;
        }
        KeyCode::Char('x') => {
            app.toggle_closed();
            return;
        }
        KeyCode::Char('S') => {
            app.open_sort_picker();
            return;
        }
        _ => {}
    }

    match app.focus {
        Focus::Sidebar => match key.code {
            KeyCode::Char('j') | KeyCode::Down => app.sidebar_move(1),
            KeyCode::Char('k') | KeyCode::Up => app.sidebar_move(-1),
            KeyCode::Char('d') if ctrl => app.sidebar_move(10),
            KeyCode::Char('u') if ctrl => app.sidebar_move(-10),
            KeyCode::PageDown => app.sidebar_move(10),
            KeyCode::PageUp => app.sidebar_move(-10),
            KeyCode::Home => app.sidebar_jump(false),
            KeyCode::End | KeyCode::Char('G') => app.sidebar_jump(true),
            KeyCode::Enter => app.sidebar_activate(false),
            KeyCode::Char(' ') => app.sidebar_activate(true),
            KeyCode::Char('l') | KeyCode::Right => app.sidebar_expand_or_child(),
            KeyCode::Char('h') | KeyCode::Left => app.sidebar_collapse_or_parent(),
            KeyCode::Char('/') => app.open_input(InputPurpose::FilterSidebar),
            KeyCode::Esc => {
                if !app.sidebar_filter.is_empty() {
                    app.sidebar_filter.clear();
                    app.clamp_sidebar();
                }
            }
            _ => {}
        },
        Focus::Tasks => {
            let page = app.layout.tasks.height.saturating_sub(4).max(1) as i32;
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => app.tasks_move(1),
                KeyCode::Char('k') | KeyCode::Up => app.tasks_move(-1),
                KeyCode::Char('d') if ctrl => app.tasks_move(page / 2),
                KeyCode::Char('u') if ctrl => app.tasks_move(-page / 2),
                KeyCode::PageDown => app.tasks_move(page),
                KeyCode::PageUp => app.tasks_move(-page),
                KeyCode::Home => app.tasks_jump(false),
                KeyCode::End | KeyCode::Char('G') => app.tasks_jump(true),
                KeyCode::Enter => app.open_detail(),
                KeyCode::Char(' ') => app.toggle_fold(),
                KeyCode::Char('l') | KeyCode::Right => {
                    if !app.unfold_current() {
                        app.open_detail();
                    }
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    if !app.fold_current() && app.show_sidebar {
                        app.focus = Focus::Sidebar;
                    }
                }
                KeyCode::Char('-') => app.fold_all_tasks(true),
                KeyCode::Char('=') | KeyCode::Char('+') => app.fold_all_tasks(false),
                KeyCode::Char('/') => app.open_input(InputPurpose::FilterTasks),
                KeyCode::Esc => {
                    if !app.task_filter.is_empty() {
                        app.task_filter.clear();
                        app.rebuild_rows();
                    } else if !app.clear_filters() && app.detail_open {
                        app.close_detail();
                    }
                }
                _ => task_action_key(app, key),
            }
        }
        Focus::Detail => {
            let page = app.detail_view_height.saturating_sub(1).max(1) as i32;
            match key.code {
                KeyCode::Esc => {
                    if app.visual.is_some() {
                        app.visual = None;
                    } else {
                        app.close_detail();
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => app.cursor_move_row(1),
                KeyCode::Char('k') | KeyCode::Up => app.cursor_move_row(-1),
                KeyCode::Char('h') | KeyCode::Left => app.cursor_move_col(-1),
                KeyCode::Char('l') | KeyCode::Right => app.cursor_move_col(1),
                KeyCode::Char('0') | KeyCode::Char('^') | KeyCode::Home => app.cursor_line_start(),
                KeyCode::Char('$') | KeyCode::End => app.cursor_line_end(),
                KeyCode::Char('w') => app.cursor_word(true),
                KeyCode::Char('d') if ctrl => app.cursor_move_row(page / 2),
                KeyCode::Char('u') if ctrl => app.cursor_move_row(-page / 2),
                KeyCode::Char('f') if ctrl => app.cursor_move_row(page),
                KeyCode::Char('b') if ctrl => app.cursor_move_row(-page),
                KeyCode::PageDown | KeyCode::Char(' ') => app.cursor_move_row(page),
                KeyCode::PageUp => app.cursor_move_row(-page),
                KeyCode::Char('G') => app.cursor_jump(true),
                KeyCode::Char('v') => app.visual_toggle(false),
                KeyCode::Char('V') => app.visual_toggle(true),
                KeyCode::Char('o') if app.visual.is_some() => app.visual_swap(),
                KeyCode::Char('y') if app.visual.is_some() => app.visual_yank(),
                KeyCode::Enter if app.visual.is_some() => app.visual_yank(),
                KeyCode::Char(']') | KeyCode::Char('J') => {
                    app.tasks_move(1);
                    app.pending_detail = None;
                    if let Some(id) = app.detail_id.clone() {
                        app.load_detail(&id, false);
                    }
                }
                KeyCode::Char('[') | KeyCode::Char('K') => {
                    app.tasks_move(-1);
                    app.pending_detail = None;
                    if let Some(id) = app.detail_id.clone() {
                        app.load_detail(&id, false);
                    }
                }
                _ => {
                    if app.visual.is_none() {
                        task_action_key(app, key);
                    }
                }
            }
        }
    }
}

/// Keys that act on the current task, shared by the table and the detail pane.
fn task_action_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s') => app.open_status_picker(),
        KeyCode::Char('p') => app.open_priority_picker(),
        KeyCode::Char('a') => app.open_assignee_picker(),
        KeyCode::Char('m') => app.toggle_assign_me(),
        KeyCode::Char('c') => app.start_comment(),
        KeyCode::Char('e') => {
            if let Some(id) = app.current_task_id() {
                app.open_input(InputPurpose::Rename { task_id: id });
            }
        }
        KeyCode::Char('E') => app.start_edit_description(),
        KeyCode::Char('d') => {
            if let Some(id) = app.current_task_id() {
                app.open_input(InputPurpose::DueDate { task_id: id });
            }
        }
        KeyCode::Char('t') => {
            if let Some(id) = app.current_task_id() {
                app.open_input(InputPurpose::Tags { task_id: id });
            }
        }
        KeyCode::Char('f') => app.open_filter_menu(),
        KeyCode::Char('n') => app.start_new_task(false),
        KeyCode::Char('N') => app.start_new_task(true),
        KeyCode::Char('o') => app.open_in_browser(),
        KeyCode::Char('y') => app.yank(true),
        KeyCode::Char('Y') => app.yank(false),
        _ => {}
    }
}

pub fn handle_mouse(app: &mut App, m: MouseEvent) {
    if !matches!(app.mode, Mode::Normal) {
        return;
    }
    let pos = Position::new(m.column, m.row);
    let in_sidebar = app.show_sidebar && app.layout.sidebar.contains(pos);
    let in_tasks = app.layout.tasks.contains(pos);
    let in_detail = app.detail_open && app.layout.detail.contains(pos);
    match m.kind {
        MouseEventKind::ScrollDown => {
            if in_sidebar {
                app.sidebar_move(1);
            } else if in_tasks {
                app.tasks_move(1);
            } else if in_detail {
                app.detail_scroll_by(3);
            }
        }
        MouseEventKind::ScrollUp => {
            if in_sidebar {
                app.sidebar_move(-1);
            } else if in_tasks {
                app.tasks_move(-1);
            } else if in_detail {
                app.detail_scroll_by(-3);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if in_sidebar {
                app.focus = Focus::Sidebar;
                let rel = m.row.saturating_sub(app.layout.sidebar.y + 1) as usize;
                let idx = rel + app.sidebar_state.offset();
                let rows = app.sidebar_rows();
                if let Some(row) = rows.get(idx) {
                    if row.selectable() {
                        let already = app.sidebar_state.selected() == Some(idx);
                        app.sidebar_state.select(Some(idx));
                        if already || matches!(row.kind, RowKind::List { .. } | RowKind::Virtual(_)) {
                            app.sidebar_activate(true);
                        }
                    }
                }
            } else if in_tasks {
                app.focus = Focus::Tasks;
                // border + header row
                if m.row >= app.layout.tasks.y + 2 {
                    let rel = (m.row - app.layout.tasks.y - 2) as usize;
                    let idx = rel + app.table_state.offset();
                    if idx < app.rows.len() && !app.rows[idx].is_header() {
                        let already = app.table_state.selected() == Some(idx);
                        app.table_state.select(Some(idx));
                        app.sync_detail_to_selection();
                        if already {
                            if app.rows[idx].child_count > 0 {
                                app.toggle_fold();
                            } else {
                                app.open_detail();
                            }
                        }
                    }
                }
            } else if in_detail {
                app.focus = Focus::Detail;
                app.detail_click(m.column, m.row);
            }
        }
        _ => {}
    }
}
