use crate::notes::NoteKind;
use crate::queue::Job;
use crate::review::{Decision, Proposal};
use crate::tui::app::{App, WriteField, WriteForm};
use crate::tui::keys::Screen;
use crate::tui::text::{short_date, truncate};
use chrono::Utc;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    let has_write = app.write_form().is_some();
    let has_promote = app.promote_title().is_some();
    let has_confirm = app.confirm_delete_count().is_some();
    let has_reason = app.review_reason().is_some();
    let has_confirm_merge = app.confirm_merge_counts().is_some();
    let has_confirm_discard = app.confirm_discard();
    let has_msg = app.message().is_some();
    let mut constraints = vec![Constraint::Length(1), Constraint::Min(0)];
    if has_write {
        constraints.push(Constraint::Length(4));
    }
    if has_promote {
        constraints.push(Constraint::Length(3));
    }
    if has_confirm {
        constraints.push(Constraint::Length(1));
    }
    if has_reason {
        constraints.push(Constraint::Length(1));
    }
    if has_confirm_merge {
        constraints.push(Constraint::Length(1));
    }
    if has_confirm_discard {
        constraints.push(Constraint::Length(1));
    }
    if has_msg {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));

    let chunks = Layout::vertical(constraints).split(area);
    let header_area = chunks[0];
    let main_area = chunks[1];

    draw_header(frame, app, header_area);

    match app.screen() {
        Screen::Inbox => draw_inbox(frame, app, main_area),
        Screen::Review => draw_review(frame, app, main_area),
        Screen::Failed => draw_failed(frame, app, main_area),
    }

    let mut idx = 2;
    if has_write {
        draw_write_form(frame, app, chunks[idx]);
        idx += 1;
    }
    if has_promote {
        draw_promote_title(frame, app, chunks[idx]);
        idx += 1;
    }
    if let Some(count) = app.confirm_delete_count() {
        let p = Paragraph::new(format!("{count} 件消します。よろしいですか [y/N]"))
            .style(Style::default().fg(app.settings().theme.error));
        frame.render_widget(p, chunks[idx]);
        idx += 1;
    }
    if has_reason {
        draw_review_reason(frame, app, chunks[idx]);
        idx += 1;
    }
    if let Some((adopted, rejected)) = app.confirm_merge_counts() {
        let p = Paragraph::new(format!(
            "採用 {adopted} 件、却下 {rejected} 件を main に入れます。よろしいですか [y/N]"
        ))
        .style(Style::default().fg(app.settings().theme.error));
        frame.render_widget(p, chunks[idx]);
        idx += 1;
    }
    if has_confirm_discard {
        let p = Paragraph::new("この失敗を捨てます。よろしいですか [y/N]")
            .style(Style::default().fg(app.settings().theme.error));
        frame.render_widget(p, chunks[idx]);
        idx += 1;
    }
    if has_msg {
        if let Some(msg) = app.message() {
            let p = Paragraph::new(msg).style(Style::default().fg(app.settings().theme.error));
            frame.render_widget(p, chunks[idx]);
        }
        idx += 1;
    }
    draw_footer(frame, app, chunks[idx]);

    if app.hub_picker().is_some() {
        draw_hub_picker(frame, app, area);
    }
    if app.show_help() {
        draw_help(frame, app, area);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let style = Style::default()
        .fg(app.settings().theme.header)
        .add_modifier(Modifier::BOLD);
    if app.screen() == Screen::Review {
        let (links, hubs) = app.review_counts();
        let left = format!(
            "review {} 件  (links {} / hub {})",
            app.proposals().len(),
            links,
            hubs
        );
        let text = if let Some(Proposal::Link { similarity, .. }) = app.current_proposal() {
            let right = format!("(近さ {:.2})", similarity);
            let total_w = area.width as usize;
            let lw = left.width();
            let rw = right.width();
            if lw + 1 + rw <= total_w {
                let pad = " ".repeat(total_w - lw - rw);
                format!("{left}{pad}{right}")
            } else {
                truncate(&format!("{left} {right}"), total_w)
            }
        } else {
            left
        };
        frame.render_widget(Paragraph::new(text).style(style), area);
        return;
    }
    let now = Utc::now();
    let text = format!(
        "inbox {} 件 (30日超 {})     review {} 件     失敗 {} 件",
        app.inbox().len(),
        app.old_count(now),
        app.review_count(),
        app.failed_count()
    );
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_inbox(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 80 {
        draw_inbox_list(frame, app, area);
    } else {
        let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
        draw_inbox_list(frame, app, cols[0]);
        draw_inbox_detail(frame, app, cols[1]);
    }
}

fn draw_inbox_list(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered();

    if app.inbox().is_empty() {
        let empty = Paragraph::new("整理されていないメモはありません")
            .style(Style::default().fg(theme.dim))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let now = Utc::now();

    let items: Vec<ListItem> = app
        .inbox()
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_cur = idx == app.cursor();
            let is_sel = app.is_selected(item);
            let is_old = item.is_old(now);
            let is_sim = app.is_similar_to_cursor(item);

            let cursor_str = if is_cur { "> " } else { "  " };
            let sel_str = if is_sel { "[x] " } else { "[ ] " };
            let date_str = short_date(&item.slug);
            let kind_str = match item.kind {
                NoteKind::Fleeting => "F",
                NoteKind::Literature => "L",
                NoteKind::Permanent => "P",
            };
            let old_str = if is_old { " 30+" } else { "" };
            let sim_str = if is_sim { " ●" } else { "" };

            // 先頭記号・日付・末尾印の幅を差し引いて本文 1 行目の表示幅を決める
            let prefix_w = cursor_str.width() + sel_str.width() + date_str.width() + 1;
            let suffix_w = 1 + kind_str.width() + old_str.width() + sim_str.width();
            let body_w = inner_w.saturating_sub(prefix_w + suffix_w);

            let first_line = item.body.lines().next().unwrap_or("");
            let body_disp = truncate(first_line, body_w);

            let mut spans = vec![
                Span::styled(
                    cursor_str,
                    if is_cur {
                        Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    sel_str,
                    if is_sel {
                        Style::default().fg(theme.selected)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(date_str, Style::default().fg(theme.dim)),
                Span::raw(" "),
                Span::styled(
                    body_disp,
                    if is_cur {
                        Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::raw(" "),
                Span::styled(kind_str, Style::default().fg(theme.dim)),
            ];
            if is_old {
                spans.push(Span::styled(old_str, Style::default().fg(theme.old)));
            }
            if is_sim {
                spans.push(Span::styled(sim_str, Style::default().fg(theme.similar)));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(block);
    let mut state = ListState::default();
    state.select(Some(app.cursor()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_inbox_detail(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered();

    let Some(item) = app.current_item() else {
        frame.render_widget(block, area);
        return;
    };

    let mut lines: Vec<Line> = item.body.lines().map(|l| Line::from(l.to_string())).collect();
    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    if item.kind == NoteKind::Literature {
        let src = item
            .source_slug
            .as_deref()
            .or(item.source_id.as_deref())
            .unwrap_or("(不明)");
        let loc = item.locator.as_deref().unwrap_or("");
        let source_line = if loc.is_empty() {
            format!("出典 {src}")
        } else {
            format!("出典 {src}  {loc}")
        };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(source_line, Style::default().fg(theme.dim))));
    }

    let similar = app.current_similar_permanent();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "似ている Permanent",
        Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
    )));
    if similar.is_empty() {
        lines.push(Line::from(Span::styled("なし", Style::default().fg(theme.dim))));
    } else {
        for near in similar {
            let title = near.title.as_deref().unwrap_or(&near.slug);
            lines.push(Line::from(format!("{title} ({:.2})", near.similarity)));
        }
    }

    let detail = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll(), 0));
    frame.render_widget(detail, area);
}

fn draw_review(frame: &mut Frame, app: &App, area: Rect) {
    let Some(prop) = app.current_proposal() else {
        draw_placeholder(frame, app, area, "提案はありません");
        return;
    };

    let (decision_area, body_area) = if app.current_decision().is_some() && area.height > 1 {
        let split = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
        (Some(split[0]), split[1])
    } else {
        (None, area)
    };

    if let (Some(d_area), Some(dec)) = (decision_area, app.current_decision()) {
        let theme = &app.settings().theme;
        let (label, color) = match dec {
            Decision::Adopt { reason: None } => ("● 採用".to_string(), theme.adopted),
            Decision::Adopt { reason: Some(r) } => (format!("● 採用 (理由: {r})"), theme.adopted),
            Decision::Reject => ("× 却下".to_string(), theme.rejected),
        };
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            d_area,
        );
    }

    match prop {
        Proposal::Link { from, to, .. } => {
            if body_area.width < 80 {
                render_review_note(frame, app, from, " 新しいノート ", body_area);
            } else {
                let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(body_area);
                render_review_note(frame, app, from, " 新しいノート ", cols[0]);
                render_review_note(frame, app, to, " 候補 ", cols[1]);
            }
        }
        Proposal::Hub { hub, note } => {
            if body_area.width < 80 {
                render_review_note(frame, app, note, " 新しいノート ", body_area);
            } else {
                let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(body_area);
                render_review_note(frame, app, note, " 新しいノート ", cols[0]);
                render_review_hub(frame, app, hub, cols[1]);
            }
        }
    }
}

fn render_review_note(frame: &mut Frame, app: &App, slug: &str, title: &str, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered().title(title);
    let mut lines = Vec::new();
    if let Some(note) = app.review_note(slug) {
        if let Some(t) = &note.title {
            lines.push(Line::from(Span::styled(
                t.clone(),
                Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
        }
        for l in note.body.lines() {
            lines.push(Line::from(l.to_string()));
        }
        if lines.is_empty() {
            lines.push(Line::from(""));
        }
    } else {
        lines.push(Line::from(Span::styled(
            slug.to_string(),
            Style::default().fg(theme.dim),
        )));
    }
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll(), 0));
    frame.render_widget(p, area);
}

fn render_review_hub(frame: &mut Frame, app: &App, hub_slug: &str, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered().title(" Hub ");
    let mut lines = Vec::new();
    if let Some(title) = app.hub_title(hub_slug) {
        lines.push(Line::from(Span::styled(
            title.to_string(),
            Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(Span::styled(
        hub_slug.to_string(),
        Style::default().fg(theme.dim),
    )));
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn draw_review_reason(frame: &mut Frame, app: &App, area: Rect) {
    let Some(input) = app.review_reason() else {
        return;
    };
    let theme = &app.settings().theme;
    let label = "理由: ";
    let label_w = label.width() as u16;
    let field_w = area.width.saturating_sub(label_w);
    let (text, col) = input.view(field_w);
    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD),
        ),
        Span::raw(text),
    ]);
    frame.render_widget(Paragraph::new(line), area);
    if area.width > 0 {
        let cx = (area.x + label_w + col).min(area.x + area.width.saturating_sub(1));
        frame.set_cursor_position((cx, area.y));
    }
}

fn draw_hub_picker(frame: &mut Frame, app: &App, area: Rect) {
    let Some(picker) = app.hub_picker() else {
        return;
    };
    let theme = &app.settings().theme;
    let w = area.width.min(50);
    let h = area.height.min((picker.hubs.len() as u16 + 2).clamp(4, 14));
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);
    let block = Block::bordered().title(" Hub を選ぶ ");
    if picker.hubs.is_empty() {
        let p = Paragraph::new("他の Hub はありません")
            .style(Style::default().fg(theme.dim))
            .block(block);
        frame.render_widget(p, popup);
        return;
    }

    let items: Vec<ListItem> = picker
        .hubs
        .iter()
        .enumerate()
        .map(|(i, hub)| {
            let is_cur = i == picker.cursor;
            let prefix = if is_cur { "> " } else { "  " };
            let style = if is_cur {
                Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{} ({})", hub.title, hub.slug),
                style,
            )))
        })
        .collect();
    let list = List::new(items).block(block);
    let mut state = ListState::default();
    state.select(Some(picker.cursor));
    frame.render_stateful_widget(list, popup, &mut state);
}

fn draw_placeholder(frame: &mut Frame, app: &App, area: Rect, label: &str) {
    let block = Block::bordered();
    let p = Paragraph::new(label)
        .style(Style::default().fg(app.settings().theme.dim))
        .block(block);
    frame.render_widget(p, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let text = app
        .usable_keys()
        .into_iter()
        .map(|(k, desc)| format!("[{k}]{desc}"))
        .collect::<Vec<_>>()
        .join(" ");
    let p = Paragraph::new(text).style(Style::default().fg(app.settings().theme.dim));
    frame.render_widget(p, area);
}

fn draw_write_form(frame: &mut Frame, app: &App, area: Rect) {
    let Some(form) = app.write_form() else {
        return;
    };
    let theme = &app.settings().theme;
    let block = Block::bordered();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_style = |focused: bool| {
        if focused {
            Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        }
    };

    let label_w = 6u16;
    let body_w = inner.width.saturating_sub(label_w);
    let (body_text, body_col) = form.body.view(body_w);
    let body_line = Line::from(vec![
        Span::styled("新規  ", label_style(form.focus == WriteField::Body)),
        Span::raw(body_text),
    ]);
    frame.render_widget(
        Paragraph::new(body_line),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if form.focus == WriteField::Body {
        let cx = (inner.x + label_w + body_col).min(inner.x + inner.width.saturating_sub(1));
        frame.set_cursor_position((cx, inner.y));
    }

    if inner.height < 2 {
        return;
    }

    let row2 = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    if app.settings().locator {
        let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(row2);
        let src_area = cols[0];
        let loc_area = cols[1];

        let src_w = src_area.width.saturating_sub(label_w + 1);
        render_source_field(frame, form, src_area, src_w, label_style(form.focus == WriteField::Source), theme.dim);
        if form.focus == WriteField::Source {
            let (_, src_col) = form.source.view(src_w);
            let cx = (src_area.x + label_w + src_col).min(src_area.x + src_area.width.saturating_sub(1));
            frame.set_cursor_position((cx, src_area.y));
        }

        let loc_w = loc_area.width.saturating_sub(label_w);
        let loc_spans = if form.locator.text().is_empty() {
            let mut s = vec![Span::styled("位置  ", label_style(form.focus == WriteField::Locator))];
            if let Some(hint) = form.locator_hint() {
                s.push(Span::styled(
                    truncate(hint, loc_w as usize),
                    Style::default().fg(theme.dim),
                ));
            }
            s
        } else {
            let (loc_text, _) = form.locator.view(loc_w);
            vec![
                Span::styled("位置  ", label_style(form.focus == WriteField::Locator)),
                Span::raw(loc_text),
            ]
        };
        frame.render_widget(Paragraph::new(Line::from(loc_spans)), loc_area);
        if form.focus == WriteField::Locator {
            let (_, loc_col) = form.locator.view(loc_w);
            let cx = (loc_area.x + label_w + loc_col).min(loc_area.x + loc_area.width.saturating_sub(1));
            frame.set_cursor_position((cx, loc_area.y));
        }
    } else {
        let src_w = row2.width.saturating_sub(label_w);
        render_source_field(frame, form, row2, src_w, label_style(form.focus == WriteField::Source), theme.dim);
        if form.focus == WriteField::Source {
            let (_, src_col) = form.source.view(src_w);
            let cx = (row2.x + label_w + src_col).min(row2.x + row2.width.saturating_sub(1));
            frame.set_cursor_position((cx, row2.y));
        }
    }
}

fn render_source_field(
    frame: &mut Frame,
    form: &WriteForm,
    area: Rect,
    field_w: u16,
    label_style: Style,
    dim_color: ratatui::style::Color,
) {
    let (src_text, _) = form.source.view(field_w);
    let mut spans = vec![
        Span::styled("出典  ", label_style),
        Span::raw(src_text.clone()),
    ];
    if let Some(rest) = form.source_suggestion() {
        let rem = (field_w as usize).saturating_sub(src_text.width());
        if rem > 0 {
            spans.push(Span::styled(
                truncate(rest, rem),
                Style::default().fg(dim_color),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_promote_title(frame: &mut Frame, app: &App, area: Rect) {
    let Some(input) = app.promote_title() else {
        return;
    };
    let theme = &app.settings().theme;
    let block = Block::bordered();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label = "題    ";
    let label_w = label.width() as u16;
    let field_w = inner.width.saturating_sub(label_w);
    let (text, col) = input.view(field_w);
    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD),
        ),
        Span::raw(text),
    ]);
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let cx = (inner.x + label_w + col).min(inner.x + inner.width.saturating_sub(1));
    frame.set_cursor_position((cx, inner.y));
}

fn draw_failed(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 80 {
        draw_failed_list(frame, app, area);
    } else {
        let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
        draw_failed_list(frame, app, cols[0]);
        draw_failed_detail(frame, app, cols[1]);
    }
}

fn draw_failed_list(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered();

    if app.failed().is_empty() {
        let empty = Paragraph::new("失敗したジョブはありません")
            .style(Style::default().fg(theme.dim))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .failed()
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let is_cur = idx == app.failed_cursor();
            let cursor_str = if is_cur { "> " } else { "  " };
            let name = entry.job.name();
            let attempts_str = format!("{}回", entry.attempts);
            let first_err = entry
                .last_error
                .as_deref()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("");
            let prefix_w = cursor_str.width() + name.width() + 1 + attempts_str.width() + 1;
            let err_w = inner_w.saturating_sub(prefix_w);
            let err_disp = truncate(first_err, err_w);

            let name_style = if is_cur {
                Style::default().fg(theme.cursor).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let spans = vec![
                Span::styled(cursor_str, name_style),
                Span::styled(name, name_style),
                Span::raw(" "),
                Span::styled(attempts_str, Style::default().fg(theme.dim)),
                Span::raw(" "),
                Span::raw(err_disp),
            ];
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(block);
    let mut state = ListState::default();
    state.select(Some(app.failed_cursor()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn job_summary(job: &Job) -> String {
    match job {
        Job::WriteNote(note) => note.body.lines().next().unwrap_or("").to_string(),
        Job::Embed { kind, slug } => {
            let kind_str = match kind {
                NoteKind::Fleeting => "Fleeting",
                NoteKind::Literature => "Literature",
                NoteKind::Permanent => "Permanent",
            };
            format!("{kind_str} {slug}")
        }
        Job::Promote { slug } => slug.clone(),
        Job::Backup { message } => message.clone(),
    }
}

fn draw_failed_detail(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.settings().theme;
    let block = Block::bordered();

    let Some(entry) = app.current_failed() else {
        frame.render_widget(block, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "エラー",
        Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
    )));
    let err_text = entry.last_error.as_deref().unwrap_or("(なし)");
    for l in err_text.lines() {
        lines.push(Line::from(l.to_string()));
    }
    if err_text.is_empty() {
        lines.push(Line::from("(なし)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "ジョブ",
        Style::default().fg(theme.header).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!("{} ({} 回)", entry.job.name(), entry.attempts)));
    lines.push(Line::from(job_summary(&entry.job)));

    let detail = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.failed_scroll(), 0));
    frame.render_widget(detail, area);
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let keys = app.help_keys();
    let key_w = keys.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let lines: Vec<Line> = keys
        .into_iter()
        .map(|(k, desc)| {
            let pad = " ".repeat(key_w.saturating_sub(k.width()) + 2);
            Line::from(format!("  {k}{pad}{desc}"))
        })
        .collect();

    let max_line_w = lines.iter().map(|l| l.width()).max().unwrap_or(20) as u16;
    let title = "キー操作 (どのキーでも閉じる)";
    let w = max_line_w
        .max(title.width() as u16)
        .saturating_add(4)
        .min(area.width);
    let h = (lines.len() as u16).saturating_add(2).min(area.height);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let popup = Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{InboxItem, NoteKind};
    use crate::tui::app::{App, Settings};
    use crate::tui::keys::Keymap;
    use crate::tui::store::FakeStore;
    use crate::tui::theme::Theme;
    use chrono::{Duration, Utc};
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;

    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            let mut x = 0;
            while x < buf.area.width {
                let sym = buf[(x, y)].symbol();
                out.push_str(sym);
                x += sym.width().max(1) as u16;
            }
            out.push('\n');
        }
        out
    }

    fn settings() -> Settings {
        Settings {
            inbox_similarity: 0.5,
            locator: false,
            theme: Theme::default(),
            keys: Keymap::default(),
        }
    }

    fn three_store() -> FakeStore {
        let now = Utc::now();
        FakeStore {
            inbox: vec![
                InboxItem {
                    kind: NoteKind::Fleeting,
                    slug: "20261117000001".into(),
                    body: "最初のメモ\n詳しい本文".into(),
                    created: now - Duration::days(31),
                    source_slug: None,
                    source_id: None,
                    locator: None,
                },
                InboxItem {
                    kind: NoteKind::Literature,
                    slug: "20261118000001".into(),
                    body: "2番目のメモ".into(),
                    created: now - Duration::days(2),
                    source_slug: Some("ahrens2017".into()),
                    source_id: None,
                    locator: Some("p.42".into()),
                },
                InboxItem {
                    kind: NoteKind::Fleeting,
                    slug: "20261119000001".into(),
                    body: "3番目のメモ".into(),
                    created: now - Duration::days(1),
                    source_slug: None,
                    source_id: None,
                    locator: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn inbox_shows_counts_and_marks() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("inbox 3 件"), "{screen}");
        assert!(screen.contains("review 0 件"), "{screen}");
        assert!(screen.contains("失敗 0 件"), "{screen}");
        assert!(screen.contains("30+"), "{screen}");
        assert!(screen.contains("[x]"), "{screen}");
        assert!(screen.contains('>'), "{screen}");
    }

    #[test]
    fn narrow_terminal_hides_right_pane() {
        let app = App::new(Box::new(three_store()), settings());
        let narrow = render(&app, 79, 20);
        assert!(!narrow.contains("詳しい本文"), "{narrow}");
        let wide = render(&app, 80, 20);
        assert!(wide.contains("詳しい本文"), "{wide}");
    }

    #[test]
    fn footer_lists_only_usable_keys() {
        let app = App::new(Box::new(three_store()), settings());
        let screen = render(&app, 100, 20);
        let footer = screen.lines().last().unwrap_or("");
        assert!(footer.contains("space"), "footer: {footer}");
        assert!(footer.contains("r"), "footer: {footer}");
        assert!(!footer.contains("[m]"), "footer: {footer}");
        assert!(!footer.contains("[a]"), "footer: {footer}");
    }

    #[test]
    fn empty_inbox_says_so() {
        let app = App::new(Box::new(FakeStore::default()), settings());
        let screen = render(&app, 100, 20);
        assert!(screen.contains("整理されていないメモはありません"), "{screen}");
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let app = App::new(Box::new(three_store()), settings());
        render(&app, 10, 3);
        render(&app, 1, 1);
    }

    #[test]
    fn write_form_shows_fields_and_hint() {
        let mut s = settings();
        s.locator = true;
        let mut app = App::new(Box::new(three_store()), s);
        app.on_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for c in "978-4-00-000000-0".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        let screen = render(&app, 100, 20);
        assert!(screen.contains("新規"), "{screen}");
        assert!(screen.contains("出典"), "{screen}");
        assert!(screen.contains("位置"), "{screen}");
        assert!(screen.contains("p.42"), "{screen}");
        let footer = screen.lines().last().unwrap_or("");
        assert!(footer.contains("Enter") && footer.contains("Tab") && footer.contains("Esc"), "{footer}");
    }

    #[test]
    fn promote_title_and_delete_confirm_render() {
        let mut app = App::new(Box::new(three_store()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("題"), "{screen}");

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        let screen2 = render(&app, 100, 20);
        assert!(screen2.contains("1 件消します。よろしいですか [y/N]"), "{screen2}");
    }

    fn review_store() -> FakeStore {
        use crate::notes::Note;
        use crate::review::{HubSummary, Proposal};

        let mut store = three_store();
        store.proposals = vec![
            Proposal::Link {
                from: "a".into(),
                to: "b".into(),
                similarity: 0.82,
            },
            Proposal::Hub {
                hub: "hub1".into(),
                note: "a".into(),
            },
        ];
        store.notes.insert(
            "a".into(),
            Note {
                kind: NoteKind::Permanent,
                slug: "a".into(),
                title: Some("ノートA".into()),
                body: "Aの本文".into(),
                created: Utc::now(),
            },
        );
        store.notes.insert(
            "b".into(),
            Note {
                kind: NoteKind::Permanent,
                slug: "b".into(),
                title: Some("ノートB".into()),
                body: "Bの本文".into(),
                created: Utc::now(),
            },
        );
        store.hubs = vec![HubSummary {
            slug: "hub1".into(),
            title: "最初のHub".into(),
        }];
        store
    }

    #[test]
    fn link_proposal_shows_both_notes_and_similarity() {
        let mut app = App::new(Box::new(review_store()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("ノートA"), "{screen}");
        assert!(screen.contains("ノートB"), "{screen}");
        assert!(screen.contains("近さ 0.82"), "{screen}");
    }

    #[test]
    fn footer_hides_keys_not_for_this_proposal() {
        let mut app = App::new(Box::new(review_store()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        let link_screen = render(&app, 100, 20);
        let link_footer = link_screen.lines().last().unwrap_or("");
        assert!(link_footer.contains("[A]"), "link_footer: {link_footer}");
        assert!(!link_footer.contains("[c]"), "link_footer: {link_footer}");

        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let hub_screen = render(&app, 100, 20);
        let hub_footer = hub_screen.lines().last().unwrap_or("");
        assert!(!hub_footer.contains("[A]"), "hub_footer: {hub_footer}");
        assert!(hub_footer.contains("[c]"), "hub_footer: {hub_footer}");
    }

    #[test]
    fn no_proposals_message() {
        let mut app = App::new(Box::new(FakeStore::default()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("提案はありません"), "{screen}");
    }

    #[test]
    fn failed_list_shows_job_and_error() {
        use crate::queue::{Entry, Job};
        use std::time::SystemTime;

        let store = FakeStore {
            failed: vec![Entry {
                id: "100-1".into(),
                job: Job::Embed {
                    kind: NoteKind::Fleeting,
                    slug: "20261117000001".into(),
                },
                attempts: 3,
                not_before: SystemTime::UNIX_EPOCH,
                last_error: Some("1 回目のエラー\n詳細".into()),
            }],
            ..Default::default()
        };
        let mut app = App::new(Box::new(store), settings());
        app.on_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("embed"), "{screen}");
        assert!(screen.contains("1 回目のエラー"), "{screen}");
    }

    #[test]
    fn help_on_review_lists_review_keys() {
        let mut app = App::new(Box::new(review_store()), settings());
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        let screen = render(&app, 100, 20);
        assert!(screen.contains("理由を書いて採用"), "{screen}");
        assert!(screen.contains("A"), "{screen}");
    }
}
