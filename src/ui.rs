//! Layout from the design: menu row on top, one title list on the left, a
//! store-page style detail pane on the right (preview, facts, actions, page).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};

use crate::app::{App, Focus, Hit, Modal, ShowRow, Status};
use crate::catalog::Title;
use crate::{md, theme};

// 26 + the 102-column preview keeps av inside 128 columns
const LIST_W: u16 = 26;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(theme::BG).fg(theme::TEXT)), area);
    let [head, body, foot] = Layout::vertical([Constraint::Length(2), Constraint::Min(5), Constraint::Length(1)]).areas(area);
    draw_header(f, app, head);
    let [list, detail] = Layout::horizontal([Constraint::Length(LIST_W), Constraint::Min(20)]).areas(body);
    let card_h = if app.focus == Focus::Show { app.show_rows().len() as u16 + 4 } else { 3 };
    let [search, card, list] = Layout::vertical([Constraint::Length(3), Constraint::Length(card_h), Constraint::Min(3)]).areas(list);
    draw_search(f, app, search);
    draw_show(f, app, card);
    draw_list(f, app, list);
    app.hits.push((detail, Hit::Detail));
    match app.current() {
        Some(t) => draw_detail(f, app, &t, detail),
        None => {
            let msg = if app.titles.is_empty() {
                "The catalog is empty."
            } else if !app.filter.is_empty() {
                "Nothing matches the search. Esc clears it."
            } else if app.show.installed || app.show.wishlist {
                "None of your titles are in the groups shown. Change what to show in the card above the list (↑ from its top)."
            } else {
                "No groups are switched on. Pick some in the card above the list (↑ from its top)."
            };
            f.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }).style(Style::default().fg(theme::DIM)).block(pane(false, "")), detail);
        }
    }
    draw_foot(f, app, foot);
    if let Some(m) = &app.modal {
        draw_modal(f, app, m, area);
    }
}

fn pane(focused: bool, title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(if focused { theme::GREEN } else { theme::LINE }))
        .title(Span::styled(title.to_string(), Style::default().fg(theme::DIM)))
        .style(Style::default().bg(theme::PANEL))
}

/// The website's wordmark, hand-lettered in half blocks.
const LOGO: [&str; 2] = ["▄▀█ █▀ █▀▀ █ █ █ █ █▀▀ █▀█ █▀ █▀▀", "█▀█ ▄█ █▄▄ █ █ ▀▄▀ ██▄ █▀▄ ▄█ ██▄"];
const SLOGAN: [&str; 2] = ["W H E R E   T E X T", "C O M E S   A L I V E"];

/// Green to amber across the wordmark, as on the website (green-hi, green at 45%, amber).
fn logo_color(x: usize, width: usize) -> ratatui::style::Color {
    let stops = [(0.0, (0x7f, 0xdc, 0x5e)), (0.45, (0x5f, 0xbf, 0x45)), (1.0, (0xff, 0xb0, 0x00))];
    let t = x as f32 / width.saturating_sub(1).max(1) as f32;
    let k = if t <= stops[1].0 { 0 } else { 1 };
    let ((t0, a), (t1, b)) = (stops[k], stops[k + 1]);
    let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * u).round() as u8;
    ratatui::style::Color::Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_widget(Block::default().style(Style::default().bg(theme::PANEL)), area);

    // top left: the wordmark, with the slogan beside it on its two rows
    let lw = LOGO[0].chars().count();
    let sw = SLOGAN.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let total = (lw + 2 + sw) as u16;
    let x0 = area.x + 1;
    for row in 0..2u16 {
        let mut spans: Vec<Span> = LOGO[row as usize]
            .chars()
            .enumerate()
            .map(|(x, c)| Span::styled(c.to_string(), Style::default().fg(logo_color(x, lw)).add_modifier(Modifier::BOLD)))
            .collect();
        spans.push(Span::raw("  "));
        spans.push(Span::styled(SLOGAN[row as usize], Style::default().fg(theme::DIM)));
        f.render_widget(Paragraph::new(Line::from(spans)), Rect { x: x0, y: area.y + row, width: total.min(area.width), height: 1 });
    }

    // right: catalog state, and how much of it the list shows
    let rx = x0 + total + 3;
    let rw = (area.x + area.width).saturating_sub(rx + 1);
    let info = if app.offline {
        Span::styled("offline: showing the cached catalog", Style::default().fg(theme::AMBER))
    } else {
        Span::styled(format!("{} titles", app.titles.len()), Style::default().fg(theme::FAINT))
    };
    f.render_widget(Paragraph::new(Line::from(info)).alignment(Alignment::Right), Rect { x: rx, y: area.y, width: rw, height: 1 });
    let shown = app.visible().len();
    if shown != app.titles.len() {
        let line = Line::from(Span::styled(format!("showing {shown}"), Style::default().fg(theme::DIM)));
        f.render_widget(Paragraph::new(line).alignment(Alignment::Right), Rect { x: rx, y: area.y + 1, width: rw, height: 1 });
    }
}

fn draw_search(f: &mut Frame, app: &mut App, area: Rect) {
    let active = app.focus == Focus::List;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if !app.filter.is_empty() { theme::GREEN } else { theme::LINE }))
        .title(Span::styled(" search ", Style::default().fg(theme::DIM)))
        .style(Style::default().bg(theme::PANEL));
    let mut spans = vec![Span::styled("/ ", Style::default().fg(theme::GREEN))];
    if app.filter.is_empty() {
        spans.push(Span::styled("type to search", Style::default().fg(theme::FAINT)));
    } else {
        spans.push(Span::styled(app.filter.clone(), Style::default().fg(theme::TEXT_HI)));
        if active {
            spans.push(Span::styled("▏", Style::default().fg(theme::GREEN)));
        }
        let n = app.visible().len();
        let count = format!(" {n}");
        let used: usize = spans.iter().map(|s| s.width()).sum();
        let room = (area.width as usize).saturating_sub(2 + used + count.len());
        spans.push(Span::raw(" ".repeat(room)));
        spans.push(Span::styled(count, Style::default().fg(theme::FAINT)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
    app.hits.push((area, Hit::Search));
}

/// The card under the search box: folded, one line saying what the list
/// shows; open, a box per kind and two that narrow to the player's titles.
fn draw_show(f: &mut Frame, app: &mut App, area: Rect) {
    let open = app.focus == Focus::Show;
    let rows = app.show_rows();
    let mut lines = Vec::new();
    let w = area.width.saturating_sub(2) as usize;
    let caret = if open { "▾ " } else { "▸ " };
    lines.push(Line::from(vec![
        Span::styled(caret, Style::default().fg(theme::GREEN)),
        Span::styled(show_summary(app, &rows, w - 2), Style::default().fg(if open { theme::TEXT_HI } else { theme::TEXT })),
    ]));
    if open {
        for (i, r) in rows.iter().enumerate() {
            if *r == ShowRow::Installed {
                let label = "─ only mine ";
                lines.push(Line::from(vec![
                    Span::styled(label, Style::default().fg(theme::FAINT)),
                    Span::styled("─".repeat(w.saturating_sub(label.chars().count())), Style::default().fg(theme::LINE)),
                ]));
            }
            let (name, count) = match r {
                ShowRow::All => ("All".to_string(), app.titles.len()),
                ShowRow::Kind(k) => (section_name(k).to_string(), app.titles.iter().filter(|t| &t.kind == k).count()),
                ShowRow::Installed => ("Installed".to_string(), app.state.installed.len()),
                ShowRow::Wishlist => ("Wishlisted".to_string(), app.state.wishlist.len()),
            };
            let on = app.row_on(r);
            let boxed = if on { "[x] " } else { "[ ] " };
            let num = count.to_string();
            let room = w.saturating_sub(4 + num.len() + 1);
            let name: String = name.chars().take(room).collect();
            let mut st = Style::default().fg(if on { theme::TEXT_HI } else { theme::DIM });
            if i == app.show_row {
                st = st.bg(theme::PANEL_HI).add_modifier(Modifier::BOLD);
            }
            lines.push(Line::from(vec![
                Span::styled(boxed, st.fg(if on { theme::GREEN } else { theme::DIM })),
                Span::styled(format!("{name:<room$} "), st),
                Span::styled(num, st.fg(theme::FAINT)),
            ]));
            app.hits.push((Rect { x: area.x + 1, y: area.y + lines.len() as u16, width: area.width.saturating_sub(2), height: 1 }, Hit::ShowRow(i)));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if open { theme::GREEN } else { theme::LINE }))
        .title(Span::styled(" show ", Style::default().fg(theme::DIM)))
        .style(Style::default().bg(theme::PANEL));
    f.render_widget(Paragraph::new(lines).block(block), area);
    // the folded line goes under the rows' hits, so a row click wins
    app.hits.insert(0, (Rect { x: area.x, y: area.y, width: area.width, height: 2 }, Hit::ShowHead));
}

/// What the list shows, in a few words: "everything", or the groups that are
/// on, then "· mine" parts.
fn show_summary(app: &App, rows: &[ShowRow], w: usize) -> String {
    let kinds: Vec<&ShowRow> = rows.iter().filter(|r| matches!(r, ShowRow::Kind(_))).collect();
    let on: Vec<&str> = kinds
        .iter()
        .filter(|r| app.row_on(r))
        .map(|r| if let ShowRow::Kind(k) = r { short_name(k) } else { "" })
        .collect();
    let mut s = match on.len() {
        0 => "nothing".to_string(),
        n if n == kinds.len() => "everything".to_string(),
        _ => on.join(", "),
    };
    match (app.show.installed, app.show.wishlist) {
        (true, true) => s += " · installed or wishlisted",
        (true, false) => s += " · installed",
        (false, true) => s += " · wishlisted",
        _ => {}
    }
    if s.chars().count() > w {
        s = s.chars().take(w.saturating_sub(1)).collect::<String>() + "…";
    }
    s
}

fn short_name(kind: &str) -> &'static str {
    match kind {
        "adventure" => "Adventures",
        "art" => "Art",
        k => section_name(k),
    }
}

fn section_name(kind: &str) -> &'static str {
    match kind {
        "game" => "Games",
        "adventure" => "Text adventures",
        "program" => "Programs",
        "art" => "Art & Demos",
        "movie" => "Movies",
        "mud" => "MUDs",
        _ => "Other",
    }
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    // the list is grouped by kind (see App::refresh); a header row starts each
    // group. Headers are drawn only: selection still counts titles, and the
    // highlighted row is mapped past the headers above it.
    let inner_w = LIST_W.saturating_sub(2) as usize;
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_row = 0;
    let mut row_title: Vec<Option<usize>> = Vec::new(); // list row to title index, None for headers
    let mut last_kind: Option<String> = None;
    for (i, t) in app.visible().iter().enumerate() {
        if last_kind.as_deref() != Some(t.kind.as_str()) {
            let label = format!("── {} ", section_name(&t.kind));
            let fill = inner_w.saturating_sub(label.chars().count());
            items.push(ListItem::new(Line::from(vec![
                Span::styled(label, Style::default().fg(theme::AMBER_SOFT).add_modifier(Modifier::BOLD)),
                Span::styled("─".repeat(fill), Style::default().fg(theme::LINE)),
            ])));
            last_kind = Some(t.kind.clone());
            row_title.push(None);
        }
        row_title.push(Some(i));
        if i == app.selected {
            selected_row = items.len();
        }
        let st = app.status_of(t);
        let color = match st {
            Status::Installed => theme::INSTALLED,
            Status::Update => theme::UPDATE,
            Status::Busy => theme::AMBER,
            Status::NotInstalled => theme::NOT_INSTALLED,
        };
        let mark = match st {
            Status::Installed => "● ",
            Status::Update => "▲ ",
            Status::Busy => "↓ ",
            Status::NotInstalled => "  ",
        };
        let (tag, tag_style) = if t.new {
            ("NEW!", Style::default().fg(theme::GREEN_HI).add_modifier(Modifier::BOLD))
        } else if t.old {
            ("OLD!", Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD))
        } else {
            ("", Style::default())
        };
        // a tag needs a space before it, so long names give way to it
        let w = LIST_W.saturating_sub(if tag.is_empty() { 5 } else { 10 }) as usize;
        let name: String = t.title.chars().take(w).collect();
        items.push(ListItem::new(Line::from(vec![
            Span::styled(mark, Style::default().fg(color)),
            Span::styled(format!("{name:<w$}"), Style::default().fg(color)),
            Span::styled(if tag.is_empty() { String::new() } else { format!(" {tag}") }, tag_style),
        ])));
    }
    let list = List::new(items)
        .block(pane(app.focus == Focus::List, " titles "))
        .highlight_style(Style::default().bg(theme::PANEL_HI).add_modifier(Modifier::BOLD));
    let mut st = ListState::default().with_selected(Some(selected_row));
    f.render_stateful_widget(list, area, &mut st);
    app.hits.push((area, Hit::List));
    let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
    for (k, row) in row_title.iter().enumerate().skip(st.offset()).take(inner.height as usize) {
        if let Some(i) = row {
            let y = inner.y + (k - st.offset()) as u16;
            app.hits.push((Rect { x: inner.x, y, width: inner.width, height: 1 }, Hit::Row(*i)));
        }
    }
}

/// Rows the scrolling column may hold below the preview; pages are short.
const COLUMN_SPARE: u16 = 400;

fn draw_detail(f: &mut Frame, app: &mut App, t: &Title, area: Rect) {
    // The whole right side is one column that scrolls: the preview, the title
    // row, then facts and page flowing on beneath it. It is drawn into a tall
    // off-screen buffer and the visible window is copied into the frame.
    let (cw, ch) = app.player.as_ref().map(|(_, p)| (p.cast.cols as u16, p.cast.rows as u16)).unwrap_or((100, 30));
    let pw = (cw + 2).min(area.width);
    let ph = ch + 2;
    let side_w = area.width.saturating_sub(pw + 1);
    let side_by_side = side_w >= 34;

    let virt = Rect { x: 0, y: 0, width: area.width, height: ph + COLUMN_SPARE };
    let mut buf = Buffer::empty(virt);
    Block::default().style(Style::default().bg(theme::PANEL)).render(virt, &mut buf);
    draw_preview(&mut buf, app, t, Rect { x: 0, y: 0, width: pw, height: ph });

    let mut text = Text::default();
    let page_y;
    // button positions in the off-screen column: (x, y, width)
    let mut button_at: Vec<(u16, u16, u16)> = Vec::new();
    let place = |x0: u16, y: u16, line: &Line, out: &mut Vec<(u16, u16, u16)>| {
        let mut x = x0;
        for (k, span) in line.spans.iter().enumerate() {
            let w = span.width() as u16;
            if k % 2 == 0 {
                out.push((x, y, w)); // even spans are buttons, odd ones the gaps
            }
            x += w;
        }
    };
    if side_by_side {
        // wide terminals: everything about the title beside the preview
        let mut lines = vec![
            Line::from(Span::styled(t.title.clone(), Style::default().fg(theme::TEXT_HI).add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(t.description.clone(), Style::default().fg(theme::DIM))),
            Line::default(),
            buttons(app, t),
        ];
        place(pw + 1, 3, &lines[3], &mut button_at);
        lines.extend(progress(app, t));
        lines.push(Line::default());
        lines.extend(facts(app, t));
        Paragraph::new(lines).wrap(Wrap { trim: true }).render(Rect { x: pw + 1, y: 0, width: side_w.saturating_sub(1), height: ph }, &mut buf);
        page_y = ph;
    } else {
        // title and buttons on one row, description under it, facts in the flow
        let b = buttons(app, t);
        let bw = b.width() as u16;
        place(pw.saturating_sub(bw), ph, &b, &mut button_at);
        Paragraph::new(Span::styled(t.title.clone(), Style::default().fg(theme::TEXT_HI).add_modifier(Modifier::BOLD)))
            .render(Rect { x: 0, y: ph, width: pw.saturating_sub(bw + 1), height: 1 }, &mut buf);
        Paragraph::new(b).alignment(Alignment::Right).render(Rect { x: 0, y: ph, width: pw, height: 1 }, &mut buf);
        let line = progress(app, t)
            .unwrap_or_else(|| Line::from(Span::styled(t.description.clone(), Style::default().fg(theme::DIM))));
        Paragraph::new(line).wrap(Wrap { trim: true }).render(Rect { x: 0, y: ph + 1, width: pw, height: 2 }, &mut buf);
        page_y = ph + 2 + u16::from(t.description.chars().count() > pw as usize);
        text.lines.extend(facts(app, t));
        text.lines.push(Line::default());
    }
    text.lines.extend(page_text(t).lines);
    let page_w = if side_by_side { area.width.saturating_sub(1) } else { pw };
    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(if app.focus == Focus::Page { theme::GREEN } else { theme::LINE })),
        )
        .render(Rect { x: 0, y: page_y, width: page_w, height: virt.height - page_y }, &mut buf);

    // the content ends at the last row with anything drawn on it
    let blank = |y: u16| (0..virt.width).all(|x| buf[(x, y)].symbol() == " ");
    let mut end = virt.height;
    while end > page_y + 1 && blank(end - 1) {
        end -= 1;
    }
    let max_scroll = end.saturating_sub(area.height);
    app.page_scroll = app.page_scroll.min(max_scroll);
    let top = app.page_scroll;

    let out = f.buffer_mut();
    for y in 0..area.height.min(virt.height - top) {
        for x in 0..area.width {
            out[(area.x + x, area.y + y)] = buf[(x, top + y)].clone();
        }
    }
    for (i, (x, y, w)) in button_at.into_iter().enumerate() {
        if y >= top && y - top < area.height {
            app.hits.push((Rect { x: area.x + x, y: area.y + y - top, width: w, height: 1 }, Hit::Button(i)));
        }
    }
    // say when there is more above or below
    let hint = |s: &str| Span::styled(s.to_string(), Style::default().fg(theme::AMBER).bg(theme::PANEL_HI));
    if top < max_scroll {
        let r = Rect { x: area.x + area.width.saturating_sub(10), y: area.y + area.height - 1, width: 10, height: 1 };
        Paragraph::new(hint(" ↓ more ")).alignment(Alignment::Right).render(r, out);
    }
    if top > 0 {
        let r = Rect { x: area.x + area.width.saturating_sub(10), y: area.y, width: 10, height: 1 };
        Paragraph::new(hint(" ↑ more ")).alignment(Alignment::Right).render(r, out);
    }
}

fn draw_preview(buf: &mut Buffer, app: &App, t: &Title, area: Rect) {
    let b = Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme::LINE));
    let inner = b.inner(area);
    b.render(area, buf);
    match &app.player {
        Some((id, p)) if *id == t.id => p.render(inner, buf),
        _ => {
            Block::default().style(Style::default().bg(crate::cast::BG)).render(inner, buf);
            if t.is_mud() {
                Paragraph::new(mud_card(t, app.lera().is_some())).alignment(Alignment::Center).render(inner, buf);
            } else {
                let msg = app.cast_note(t).unwrap_or_default();
                let y = inner.y + inner.height / 2;
                Paragraph::new(Span::styled(msg, Style::default().fg(theme::FAINT)))
                    .alignment(Alignment::Center)
                    .render(Rect { y, height: 1, ..inner }, buf);
            }
        }
    }
}

fn mud_card(t: &Title, lera: bool) -> Text<'static> {
    let mut l = vec![Line::default(); 3];
    l.push(Line::from(Span::styled(t.title.to_uppercase(), Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD))));
    if let Some(s) = t.since {
        l.push(Line::from(Span::styled(format!("online since {s}"), Style::default().fg(theme::DIM))));
    }
    l.push(Line::default());
    if let Some(c) = &t.connect {
        l.push(Line::from(Span::styled(format!("{} {}", c.host, c.port), Style::default().fg(theme::GREEN_HI))));
    }
    l.push(Line::default());
    if lera {
        l.push(Line::from(Span::styled("Connect opens the world in Lera.", Style::default().fg(theme::FAINT))));
    } else {
        l.push(Line::from(Span::styled("Install Lera, the Asciiverse MUD client, to play here.", Style::default().fg(theme::FAINT))));
        l.push(Line::from(Span::styled("Until then Connect opens the world in telnet.", Style::default().fg(theme::FAINT))));
    }
    Text::from(l)
}

fn buttons(app: &App, t: &Title) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (_, label, enabled)) in app.actions(t).iter().enumerate() {
        let sel = app.focus == Focus::Actions && app.action == i;
        let st = match (enabled, sel) {
            (false, _) => Style::default().fg(theme::FAINT).bg(theme::PANEL_HI),
            (true, true) => Style::default().fg(theme::BG).bg(theme::GREEN_HI).add_modifier(Modifier::BOLD),
            (true, false) => Style::default().fg(theme::GREEN_HI).bg(theme::PANEL_HI),
        };
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(format!(" {label} "), st));
    }
    Line::from(spans)
}

fn progress(app: &App, t: &Title) -> Option<Line<'static>> {
    app.progress.get(&t.id).map(|(done, total, msg)| {
        let pct = if *total > 0 { done * 100 / total } else { 0 };
        Line::from(Span::styled(format!("{msg} {pct}%"), Style::default().fg(theme::AMBER)))
    })
}

fn facts(app: &App, t: &Title) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let fact = |k: &str, v: String| {
        Line::from(vec![Span::styled(format!("{k:<11}"), Style::default().fg(theme::FAINT)), Span::styled(v, Style::default().fg(theme::TEXT))])
    };
    if t.is_mud() {
        if let Some(c) = &t.connect {
            lines.push(fact("Address", format!("{}:{}", c.host, c.port)));
        }
        if let Some(w) = &t.website {
            lines.push(fact("Website", w.clone()));
        }
        return lines;
    }
    if let Some(d) = &t.developer {
        lines.push(fact("Developer", d.name.clone()));
    }
    if let Some(u) = &t.upstream {
        lines.push(fact("Original", u.name.clone()));
    }
    if let Some(name) = t.collection_name(&app.collections) {
        let n = app.titles.iter().filter(|o| o.collection == t.collection).count();
        lines.push(fact("Part of", if n > 1 { format!("{name} ({n} games)") } else { name.to_string() }));
    }
    if !t.version.is_empty() {
        let inst = app.state.installed.get(&t.id).map(|i| format!("  (installed {})", i.version)).unwrap_or_default();
        lines.push(fact("Version", format!("{}{inst}", t.version)));
    }
    if !t.license.is_empty() {
        lines.push(fact("License", t.license.clone()));
    }
    if let Some(p) = app.state.played.get(&t.id) {
        let last = p.last_played.get(..10).unwrap_or("");
        lines.push(fact("Played", format!("{}, last on {last}", crate::launch::duration(p.seconds))));
    }
    if let Some(m) = t.min_terminal {
        lines.push(fact("Needs", format!("{}x{} terminal{}", m.cols, m.rows, if t.requires.mouse { ", mouse" } else { "" })));
    }
    if t.requires.gamepad {
        lines.push(fact("Gamepad", "supported".to_string()));
    }
    if !t.requires.glyphs.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Font needs {} glyphs", t.requires.glyphs.join(", ")),
            Style::default().fg(theme::AMBER_SOFT),
        )));
    }
    if !t.system_deps.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Needs system packages: {}", t.system_deps.join(", ")),
            Style::default().fg(theme::AMBER_SOFT),
        )));
    }
    if crate::catalog::pick_platform(t).is_none() && !t.platforms.is_empty() {
        lines.push(Line::from(Span::styled("No build for this platform", Style::default().fg(theme::UPDATE))));
    }
    lines
}

fn page_text(t: &Title) -> Text<'static> {
    let mut text = Text::from(provenance(t));
    if !t.page.trim().is_empty() {
        text.lines.extend(md::render(&t.page).lines);
        return text;
    }
    let mut lines = vec![Line::from(Span::styled(t.description.clone(), Style::default().fg(theme::TEXT)))];
    if !t.topics.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(t.topics.join("  ·  "), Style::default().fg(theme::FAINT))));
    }
    text.lines.extend(lines);
    text
}

// Where it comes from, above the page: who made it, what we changed, and the links
fn provenance(t: &Title) -> Vec<Line<'static>> {
    if t.provenance.is_none() && t.upstream.is_none() {
        return Vec::new();
    }
    let head = Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(Span::styled("WHERE IT COMES FROM", head))];
    if let Some(p) = &t.provenance {
        lines.push(Line::from(Span::styled(p.clone(), Style::default().fg(theme::TEXT))));
    }
    let link = |k: &str, v: String| {
        Line::from(vec![Span::styled(format!("{k:<13}"), Style::default().fg(theme::FAINT)), Span::styled(v, Style::default().fg(theme::GREEN))])
    };
    if let Some(u) = t.upstream.as_ref().filter(|u| !u.url.is_empty()) {
        lines.push(link("Original", u.url.clone()));
    }
    if let Some(r) = &t.repo {
        lines.push(link("Our source", format!("https://github.com/{r}")));
        if let Some(c) = &t.changes {
            let k = if c.contains("PATCHES") { "Our changes" } else { "Provenance" };
            lines.push(link(k, format!("https://github.com/{r}/blob/HEAD/{c}")));
        }
    }
    lines.push(Line::default());
    lines
}

fn draw_foot(f: &mut Frame, app: &App, area: Rect) {
    let hints = match app.focus {
        Focus::Show => "↑↓ move  Space switch on or off  ↓ past the end or Esc back to the list  type to search",
        Focus::List => "↑↓ select  ↑ at the top: what to show  type to search  Enter actions  Tab panels  F5 refresh  Esc quit",
        Focus::Actions => "←→ choose  Enter do it  ↑↓ back to list  Tab page  Esc back",
        Focus::Page => "↑↓ scroll  Space pause preview  Tab list  Esc back",
    };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", app.status), Style::default().fg(theme::AMBER_SOFT)),
        Span::styled(hints, Style::default().fg(theme::FAINT)),
    ]);
    f.render_widget(Paragraph::new(line).style(Style::default().bg(theme::PANEL)), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_modal(f: &mut Frame, app: &App, m: &Modal, area: Rect) {
    let (title, body, w, h): (&str, Text, u16, u16) = match m {
        Modal::Quit => (" quit ", Text::from("Leave Asciiverse?\n\nEnter to quit, any other key to stay."), 44, 7),
        Modal::Message(s) => (" note ", Text::from(format!("{s}\n\nany key to close")), 72, 6 + s.lines().count() as u16 + s.len() as u16 / 66),
        Modal::Choice { title, text, options, sel } => {
            let mut lines: Vec<Line> = text.lines().map(|l| Line::from(l.to_string())).collect();
            lines.push(Line::default());
            let mut spans = Vec::new();
            for (i, (label, _)) in options.iter().enumerate() {
                let st = if i == *sel {
                    Style::default().fg(theme::BG).bg(theme::GREEN_HI).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::GREEN_HI).bg(theme::PANEL)
                };
                spans.push(Span::styled(format!(" {label} "), st));
                spans.push(Span::raw("  "));
            }
            lines.push(Line::from(spans));
            lines.push(Line::default());
            lines.push(Line::from(Span::styled("arrows choose, Enter confirms, Esc cancels", Style::default().fg(theme::FAINT))));
            let h = 6 + text.lines().count() as u16 + text.len() as u16 / 60;
            (title.as_str(), Text::from(lines), 64, h)
        }
        Modal::Uninstall { id } => {
            let name = app.titles.iter().find(|t| t.id == *id).map(|t| t.title.clone()).unwrap_or(id.clone());
            let opt = |on: bool, s: &str| {
                Span::styled(
                    format!(" {} {s} ", if on { "◉" } else { "○" }),
                    if on { Style::default().fg(theme::BG).bg(theme::GREEN_HI) } else { Style::default().fg(theme::DIM) },
                )
            };
            let body = Text::from(vec![
                Line::from(format!("Uninstall {name}?")),
                Line::default(),
                Line::from(opt(app.uninstall_keep_data, "keep its saves and settings")),
                Line::from(opt(!app.uninstall_keep_data, "remove its saves and settings too")),
                Line::default(),
                Line::from(Span::styled("arrows choose, Enter uninstalls, Esc cancels", Style::default().fg(theme::FAINT))),
            ]);
            (" uninstall ", body, 56, 10)
        }
    };
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme::AMBER))
        .title(Span::styled(title, Style::default().fg(theme::AMBER)))
        .style(Style::default().bg(theme::PANEL_HI).fg(theme::TEXT_HI));
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }).block(b), r);
}
