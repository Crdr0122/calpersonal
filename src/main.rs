mod calendar_auth;
mod file_writing;
mod tasks_auth;
use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDate};
use chrono_tz::Tz;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use google_calendar3::{CalendarHub, api};
use google_tasks1::TasksHub;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::connect;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    layout::{Constraint, Direction, Layout},
    prelude::Stylize,
    style::{Color, Modifier},
    symbols,
    text::Text,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use rustls;
use std::collections::HashMap;
use std::io;
use std::sync::LazyLock;

static APP_TIMEZONE: LazyLock<Tz> =
    LazyLock::new(|| "Asia/Tokyo".parse().expect("Invalid Timezone"));
// Struct to hold our application state
struct App {
    tasks_visible: bool,
    events_visible: bool,
    current_date: NaiveDate, // The date being displayed
    today: NaiveDate,        // Today's date for comparison
    cursor_line: u16,
    exit: bool,

    // Calendar stuff
    event_hub: Option<CalendarHub<HttpsConnector<connect::HttpConnector>>>, // The authenticated client
    events_cache: HashMap<NaiveDate, Vec<api::Event>>, // date → events that day
    events_loading: bool,

    task_hub: Option<TasksHub<HttpsConnector<connect::HttpConnector>>>, // The authenticated client
    tasks_cache: Vec<google_tasks1::api::Task>,                         // date → events that day
    task_or_event_num: u16,

    events_update_rx: Option<tokio::sync::mpsc::Receiver<HashMap<NaiveDate, Vec<api::Event>>>>,
    tasks_update_rx: Option<tokio::sync::mpsc::Receiver<Vec<google_tasks1::api::Task>>>,
    // rt_handle: tokio::runtime::Handle, // For spawning async from sync contexts
    needs_refresh: bool,
}

impl App {
    async fn new() -> App {
        let today = Local::now().date_naive();
        let event_hub = calendar_auth::get_calendar_hub().await.ok();
        let task_hub = tasks_auth::get_tasks_hub().await.ok();
        let events_cache = file_writing::load_events_cache();
        let tasks_cache = file_writing::load_tasks_cache();
        // let rt_handle = tokio::runtime::Handle::current();
        let mut app = Self {
            current_date: today,
            today: today,
            tasks_visible: false,
            events_visible: false,
            cursor_line: 0,
            exit: false,

            event_hub: event_hub,
            events_cache,
            events_loading: false,

            task_hub: task_hub,
            tasks_cache,
            task_or_event_num: 0,

            events_update_rx: None,
            tasks_update_rx: None,
            // rt_handle,
            needs_refresh: false,
        };
        app.start_background_refresh();
        app
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        use crossterm::event::{poll, read};
        use std::time::Duration;
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            if poll(Duration::from_millis(250))? {
                match read()? {
                    Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                        self.handle_key_event(key_event);
                    }
                    _ => {}
                }
            }
            self.check_updates();
            if self.needs_refresh {
                self.start_background_refresh();
                self.needs_refresh = false;
            }
        }
        Ok(())
    }

    pub fn title(&self) -> String {
        self.current_date.format("%Y %B").to_string()
    }

    fn first_day_of_month(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.current_date.year(), self.current_date.month(), 1).unwrap()
    }

    fn last_day_of_month(&self) -> NaiveDate {
        let first_day = self.first_day_of_month();
        first_day
            .checked_add_months(Months::new(1))
            .unwrap()
            .pred_opt()
            .unwrap()
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn generate_calendar_grid(&self) -> (Vec<Vec<(NaiveDate, bool, bool)>>, usize) {
        let first_day = self.first_day_of_month();
        let last_day = self.last_day_of_month();
        let current_month = self.current_date.month();

        // Get weekday of first day (0 = Sunday, 6 = Saturday)
        let first_weekday = first_day.weekday().num_days_from_sunday() as i32;

        // Calculate starting date (might be from previous month)
        let start_date = first_day - chrono::Duration::days(first_weekday as i64);
        let number_of_days = last_day.signed_duration_since(start_date).num_days();
        let number_of_rows = if number_of_days > 34 {
            6
        } else if number_of_days < 29 {
            4
        } else {
            5
        };

        let mut grid = Vec::new();

        // Generate 6 weeks (42 days total)
        for week in 0..6 {
            let mut week_days = Vec::new();
            for day in 0..7 {
                let drawing_date = start_date + chrono::Duration::days((week * 7 + day) as i64);
                // Check if this date is in the current month
                let is_current_month = drawing_date.month() == current_month;

                // Check if this date is today
                let is_today = drawing_date == self.today;
                week_days.push((drawing_date, is_current_month, is_today));
            }
            grid.push(week_days);
        }
        (grid, number_of_rows)
    }

    fn start_background_refresh(&mut self) {
        if let Some(hub) = self.event_hub.clone() {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.events_update_rx = Some(rx);
            self.events_loading = true;
            tokio::spawn(async move {
                if let Some(new_events) = App::fetch_events(&hub).await {
                    file_writing::save_events_cache(&new_events);
                    let _ = tx.send(new_events).await;
                }
            });
        }
        if let Some(hub) = self.task_hub.clone() {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.tasks_update_rx = Some(rx);
            tokio::spawn(async move {
                if let Some(new_tasks) = App::fetch_tasks(&hub).await {
                    file_writing::save_tasks_cache(&new_tasks);
                    let _ = tx.send(new_tasks).await;
                }
            });
        }
    }

    fn check_updates(&mut self) {
        if let Some(rx) = &mut self.events_update_rx {
            if let Ok(new_cache) = rx.try_recv() {
                self.events_cache = new_cache;
            }
        }
        if let Some(rx) = &mut self.tasks_update_rx {
            if let Ok(new_cache) = rx.try_recv() {
                self.tasks_cache = new_cache;
            }
        }
        self.events_loading = false;
    }

    async fn fetch_events(
        hub: &CalendarHub<HttpsConnector<connect::HttpConnector>>,
    ) -> Option<HashMap<NaiveDate, Vec<api::Event>>> {
        match hub
            .events()
            .list("primary")
            .single_events(true)
            .order_by("startTime")
            // Optional: Add time bounds for efficiency, e.g., .time_min(Local::now() - Months::new(1)), .time_max(Local::now() + Months::new(6))
            .doit()
            .await
        {
            Ok((_, events_list)) => {
                let mut map: HashMap<NaiveDate, Vec<api::Event>> = HashMap::new();
                if let Some(items) = events_list.items {
                    for event in items {
                        let start_date_and_event = if let Some(start) = &event.start {
                            if let Some(date_time_str) = start.date_time {
                                // Convert to your local timezone and get the local date + time
                                let local_dt = date_time_str.with_timezone(&*APP_TIMEZONE);
                                Some(local_dt.date_naive())
                            } else if let Some(date_str) = start.date {
                                Some(date_str)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Some(start_date) = start_date_and_event {
                            map.entry(start_date).or_default().push(event);
                        }
                    }
                }
                Some(map)
            }
            Err(e) => {
                eprintln!("Failed to fetch events: {e:?}");
                None
            }
        }
    }

    async fn fetch_tasks(
        hub: &TasksHub<HttpsConnector<connect::HttpConnector>>,
    ) -> Option<Vec<google_tasks1::api::Task>> {
        let tasklists = match hub.tasklists().list().doit().await {
            Ok((_, tasks_list)) => tasks_list.items.unwrap_or_default(),
            Err(e) => {
                eprintln!("Failed to fetch tasklists: {e:?}");
                return None;
            }
        };
        let mut all_tasks = Vec::new();
        for tasklist in tasklists {
            if let Some(id) = tasklist.id {
                match hub.tasks().list(&id).doit().await {
                    Ok((_, tasks)) => {
                        if let Some(items) = tasks.items {
                            all_tasks.extend(items);
                        }
                    }
                    Err(e) => eprintln!("Failed to fetch tasks for list {id}: {e:?}"),
                }
            }
        }
        Some(all_tasks)
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Char('h') => self.move_left(),
            KeyCode::Char('l') => self.move_right(),
            KeyCode::Char('k') => self.move_up(),
            KeyCode::Char('j') => self.move_down(),
            KeyCode::Char('>') => {
                self.current_date = self
                    .current_date
                    .checked_add_months(Months::new(1))
                    .unwrap()
            }
            KeyCode::Char('<') => {
                self.current_date = self
                    .current_date
                    .checked_sub_months(Months::new(1))
                    .unwrap()
            }
            KeyCode::Char('E') => self.toggle_event_visibility(),
            KeyCode::Char('T') => self.toggle_tasks_visibility(),
            KeyCode::Char('R') => self.needs_refresh = true,
            _ => {}
        }
    }

    fn exit(&mut self) {
        if self.events_visible {
            self.events_visible = false;
        } else {
            self.exit = true;
        }
    }

    fn move_right(&mut self) {
        if self.tasks_visible || self.events_visible {
            return;
        }
        self.current_date = self.current_date.succ_opt().unwrap();
    }

    fn move_left(&mut self) {
        if self.tasks_visible || self.events_visible {
            return;
        }
        self.current_date = self.current_date.pred_opt().unwrap();
    }

    fn move_up(&mut self) {
        if self.tasks_visible || self.events_visible {
            if self.cursor_line > 0 {
                self.cursor_line = self.cursor_line - 1;
            }
        } else {
            self.current_date = self.current_date.checked_sub_days(Days::new(7)).unwrap();
        }
    }

    fn move_down(&mut self) {
        if self.tasks_visible || self.events_visible {
            if self.cursor_line < self.task_or_event_num {
                self.cursor_line = self.cursor_line - 1;
            }
        } else {
            self.current_date = self.current_date.checked_add_days(Days::new(7)).unwrap();
        }
    }

    fn toggle_event_visibility(&mut self) {
        self.events_visible = !self.events_visible;
        self.cursor_line = 1;
    }
    fn toggle_tasks_visibility(&mut self) {
        self.tasks_visible = !self.tasks_visible;
        self.cursor_line = 1;
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let main_area = if self.tasks_visible {
            Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([70, 30]),
            )
            .split(area)
        } else {
            Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([100, 0]),
            )
            .split(area)
        };
        let main_chunks = Layout::new(
            Direction::Vertical,
            Constraint::from_percentages([3, 94, 3]),
        )
        .split(main_area[0]);

        // Title area
        let title_area = Layout::new(
            Direction::Horizontal,
            Constraint::from_percentages([3, 94, 3]),
        )
        .split(main_chunks[0]);

        Paragraph::new(self.title())
            .centered()
            .style(Modifier::BOLD)
            .render(title_area[1], buf);

        if self.events_loading {
            Paragraph::new("⟳")
                .centered()
                .style(Modifier::BOLD)
                .render(title_area[0], buf);
        }

        // Calendar area
        let calendar_area = main_chunks[1];
        let (drawn_dates, number_of_rows) = self.generate_calendar_grid();
        let height = (calendar_area.height as usize) / (number_of_rows);

        let mut calendar_row_constraints = vec![Constraint::Length(height as u16); number_of_rows];
        calendar_row_constraints.insert(0, Constraint::Length(calendar_area.height / 11));
        let calendar_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(calendar_row_constraints)
            .split(calendar_area);

        // Calendar Header
        let weekday_area = calendar_rows[0];
        let weekday_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 7); 7])
            .split(weekday_area);

        let left_bottom_border_cross = symbols::border::Set {
            bottom_left: symbols::line::NORMAL.cross,
            top_left: symbols::line::NORMAL.horizontal_down,
            ..symbols::border::PLAIN
        };
        let left_bottom_border = symbols::border::Set {
            bottom_left: symbols::line::NORMAL.vertical_right,
            ..symbols::border::PLAIN
        };
        let left_border = symbols::border::Set {
            bottom_left: symbols::line::NORMAL.horizontal_up,
            top_left: symbols::line::NORMAL.horizontal_down,
            ..symbols::border::PLAIN
        };
        let right_bottom_border = symbols::border::Set {
            bottom_left: symbols::line::NORMAL.cross,
            top_left: symbols::line::NORMAL.horizontal_down,
            bottom_right: symbols::line::NORMAL.vertical_left,
            ..symbols::border::PLAIN
        };
        let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        for (i, &day) in weekdays.iter().enumerate() {
            let cell_border = Block::default();
            if i == 0 {
                // Sunday
                let name = Text::styled(day, Color::Red);
                let cell = Paragraph::new(name).centered();
                let day_block = cell_border
                    .borders(Borders::BOTTOM | Borders::TOP | Borders::LEFT)
                    .border_set(left_bottom_border);
                cell.block(day_block).render(weekday_cols[i], buf)
            } else if i == 6 {
                // Saturday
                let name = Text::styled(day, Color::Blue);
                let cell = Paragraph::new(name).centered();
                let day_block = cell_border
                    .borders(Borders::ALL)
                    .border_set(right_bottom_border);
                cell.block(day_block).render(weekday_cols[i], buf)
            } else {
                // Weekdays
                let cell = Paragraph::new(day).centered();
                let day_block = cell_border
                    .borders(Borders::BOTTOM | Borders::TOP | Borders::LEFT)
                    .border_set(left_bottom_border_cross);
                cell.block(day_block).render(weekday_cols[i], buf)
            }
        }

        // Days Area
        let cursor_date = self.current_date.day();

        for (row_index, row_chunk) in calendar_rows[1..(number_of_rows + 1)].iter().enumerate() {
            let horizontal_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 7); 7])
                .split(*row_chunk);

            // Draw each cell in this row
            for (col_index, cell_chunk) in horizontal_chunks.iter().enumerate() {
                let cell_border = Block::default();
                let current_cell = drawn_dates[row_index][col_index];
                let current_date = current_cell.0.day();
                let is_cursor_here = cursor_date == current_date && current_cell.1;
                let focus_not_on_calendar = self.tasks_visible || self.events_visible;
                let day = if is_cursor_here && (!focus_not_on_calendar) {
                    ratatui::widgets::ListItem::new(format!("{}{:<30}", current_date, " "))
                        .on_dark_gray()
                } else {
                    ratatui::widgets::ListItem::new(format!("{}", current_date))
                };

                let empty_vec = &vec![];
                let today_events = self.events_cache.get(&current_cell.0).unwrap_or(empty_vec);

                let mut items: Vec<ratatui::widgets::ListItem> = if today_events.is_empty() {
                    vec![]
                } else {
                    today_events
                        .iter()
                        .map(|ev| {
                            let title = ev.summary.as_deref().unwrap_or("Untitled");
                            let time = ev
                                .start
                                .as_ref()
                                .and_then(|s| s.date_time)
                                .map(|dt| {
                                    dt.with_timezone(&*APP_TIMEZONE)
                                        .format("%H:%M ")
                                        .to_string()
                                })
                                .unwrap_or("".to_string());
                            let e = if current_cell.1 {
                                Text::raw(format!("{time}{title}"))
                            } else {
                                Text::raw(format!("{time}{title}")).dark_gray()
                            };
                            ratatui::widgets::ListItem::new(e)
                        })
                        .collect()
                };

                if col_index == 0 {
                    // Sunday
                    let day = if current_cell.2 {
                        day.green()
                    } else if current_cell.1 {
                        day.red()
                    } else {
                        day.dark_gray()
                    };
                    items.insert(0, day);
                    let cell = ratatui::widgets::List::new(items);
                    let day_block = cell_border.borders(Borders::BOTTOM | Borders::LEFT);
                    let day_block = if row_index == number_of_rows - 1 {
                        day_block
                    } else {
                        day_block.border_set(left_bottom_border)
                    };
                    cell.block(day_block).render(*cell_chunk, buf)
                } else if col_index == 6 {
                    // Saturday
                    let day = if current_cell.2 {
                        day.green()
                    } else if current_cell.1 {
                        day.blue()
                    } else {
                        day.dark_gray()
                    };
                    items.insert(0, day);
                    let cell = ratatui::widgets::List::new(items);
                    let day_block =
                        cell_border.borders(Borders::BOTTOM | Borders::RIGHT | Borders::LEFT);
                    let day_block = if row_index == number_of_rows - 1 {
                        day_block.border_set(left_border)
                    } else {
                        day_block.border_set(right_bottom_border)
                    };
                    cell.block(day_block).render(*cell_chunk, buf)
                } else {
                    // Weekdays
                    let day = if current_cell.2 {
                        day.green()
                    } else if current_cell.1 {
                        day
                    } else {
                        day.dark_gray()
                    };
                    items.insert(0, day);
                    let cell = ratatui::widgets::List::new(items);
                    let day_block = cell_border.borders(Borders::BOTTOM | Borders::LEFT);

                    let day_block = if row_index == number_of_rows - 1 {
                        day_block.border_set(left_border)
                    } else {
                        day_block.border_set(left_bottom_border_cross)
                    };
                    cell.block(day_block).render(*cell_chunk, buf)
                }
            }
        }
        if self.events_visible {
            let event_area_horizontal = Layout::new(
                Direction::Vertical,
                Constraint::from_percentages([16, 68, 16]),
            )
            .split(main_area[0]);
            let event_area = Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([20, 60, 20]),
            )
            .split(event_area_horizontal[1]);
            Clear::default().render(event_area[1], buf);

            let empty_vec = &vec![];
            let today_events = self
                .events_cache
                .get(&self.current_date)
                .unwrap_or(empty_vec);

            let items: Vec<ratatui::widgets::ListItem> = if today_events.is_empty() {
                vec![]
            } else {
                today_events
                    .iter()
                    .map(|ev| {
                        let title = ev.summary.as_deref().unwrap_or("Untitled");
                        let start_time = ev
                            .start
                            .as_ref()
                            .and_then(|s| s.date_time)
                            .map(|dt| {
                                dt.with_timezone(&*APP_TIMEZONE)
                                    .format(" %H:%M ")
                                    .to_string()
                            })
                            .unwrap_or(" ".to_string());
                        let end_time = ev
                            .end
                            .as_ref()
                            .and_then(|s| s.date_time)
                            .map(|dt| {
                                dt.with_timezone(&*APP_TIMEZONE)
                                    .format("- %H:%M ")
                                    .to_string()
                            })
                            .unwrap_or("".to_string());
                        ratatui::widgets::ListItem::new(format!("{start_time}{end_time}{title}"))
                    })
                    .collect()
            };

            ratatui::widgets::List::new(items)
                .block(Block::bordered().title("Events"))
                .render(event_area[1], buf);
        }

        if self.tasks_visible {
            let task_area = Layout::new(
                Direction::Vertical,
                Constraint::from_percentages([2, 96, 2]),
            )
            .margin(4)
            .split(main_area[1]);

            let tasks = self.tasks_cache.clone();
            let items: Vec<Text> = {
                tasks
                    .iter()
                    .map(|ev| {
                        let title = ev.title.as_deref().unwrap_or("Untitled");
                        let time = match ev.due.as_deref() {
                            Some(duedate) => match DateTime::parse_from_rfc3339(duedate) {
                                Ok(e) => e.date_naive().format("%Y/%m/%d ").to_string(),
                                Err(_) => "".to_string(),
                            },
                            None => "".to_string(),
                        };
                        match ev.completed {
                            Some(_) => Text::raw(format!("{time}{title}")).dark_gray(),
                            None => Text::raw(format!("{time}{title}")),
                        }
                    })
                    .collect()
            };

            ratatui::widgets::List::new(items)
                .block(Block::bordered().title("Tasks".bold().into_centered_line()))
                .render(task_area[1], buf);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), io::Error> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install awc_ls_rs crypto provider");

    let mut terminal = ratatui::init();
    let mut calendar_init = App::new().await;
    let res = calendar_init.run(&mut terminal);
    ratatui::restore();
    res
}
