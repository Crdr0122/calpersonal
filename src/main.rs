mod calendar_auth;
mod file_writing;
mod parse_input;
mod tasks_auth;
use chrono::{DateTime, Datelike, Days, FixedOffset, Local, Months, NaiveDate};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use google_calendar3::{CalendarHub, api};
use google_tasks1::{TasksHub, api::Task};
use hyper_util::client::legacy::connect;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    layout::{Constraint, Direction, Layout},
    prelude::Stylize,
    style::{Color, Modifier},
    symbols,
    text::{Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use rustls;
use std::collections::HashMap;
use std::io;

struct App {
    app_layout: AppLayout,
    current_date: NaiveDate, // The date being displayed
    today: NaiveDate,        // Today's date for comparison
    cursor_line: usize,
    app_tz: FixedOffset,
    exit: bool,

    // Calendar stuff
    event_hub: Option<CalendarHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>>, // The authenticated client
    events_cache: HashMap<NaiveDate, Vec<(api::Event, String)>>, // date → events that day
    task_hub: Option<TasksHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>>, // The authenticated client
    tasks_cache: Vec<(Task, String)>, // date → events that day

    change_feedback_tx: Option<tokio::sync::mpsc::Sender<(String, StatusColor)>>,
    change_feedback_rx: Option<tokio::sync::mpsc::Receiver<(String, StatusColor)>>,
    refreshing_status: (String, StatusColor),
    changing_status: (String, StatusColor),

    inputting: bool,
    cursor_index: usize,
    input_buffer: String,
    updating_event_or_task: bool,

    events_update_rx:
        Option<tokio::sync::mpsc::Receiver<HashMap<NaiveDate, Vec<(api::Event, String)>>>>,
    tasks_update_rx: Option<tokio::sync::mpsc::Receiver<Vec<(Task, String)>>>,
    needs_refresh: bool,

    auth_status: AuthStatus,

    // Channels to receive hubs when auth completes
    calendar_hub_rx: Option<
        tokio::sync::oneshot::Receiver<
            Option<CalendarHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>>,
        >,
    >,
    tasks_hub_rx: Option<
        tokio::sync::oneshot::Receiver<
            Option<TasksHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>>,
        >,
    >,
}

#[derive(PartialEq)]
enum AuthStatus {
    Authenticating,
    Online,
    Offline, // Failed or no internet
}

#[derive(Clone)]
enum StatusColor {
    Green,
    Yellow,
    Red,
    White,
}
enum AppLayout {
    Calendar,
    Events,
    Tasks(bool),
}

impl App {
    async fn new() -> App {
        let today = Local::now().date_naive();
        let app_tz = Local::now().offset().clone();
        let events_cache = file_writing::load_events_cache();
        let tasks_cache = file_writing::load_tasks_cache();
        let (calendar_tx, calendar_rx) = tokio::sync::oneshot::channel();
        let (tasks_tx, tasks_rx) = tokio::sync::oneshot::channel();
        let rt_handle = tokio::runtime::Handle::current();
        let (deletion_feedback_tx, deletion_feedback_rx) = tokio::sync::mpsc::channel(1);
        rt_handle.spawn(async move {
            let hub = calendar_auth::get_calendar_hub().await.ok();
            let _ = calendar_tx.send(hub);
        });

        rt_handle.spawn(async move {
            let hub = tasks_auth::get_tasks_hub().await.ok();
            let _ = tasks_tx.send(hub);
        });
        let app = Self {
            current_date: today,
            today: today,
            app_layout: AppLayout::Calendar,
            cursor_line: 0,
            app_tz,
            exit: false,

            event_hub: None,
            events_cache,
            task_hub: None,
            tasks_cache,
            refreshing_status: (String::new(), StatusColor::White),
            changing_status: (String::new(), StatusColor::White),

            change_feedback_tx: Some(deletion_feedback_tx),
            change_feedback_rx: Some(deletion_feedback_rx),

            inputting: false,
            cursor_index: 0,
            input_buffer: String::new(),
            updating_event_or_task: false,

            events_update_rx: None,
            tasks_update_rx: None,
            needs_refresh: false,

            auth_status: AuthStatus::Authenticating,
            calendar_hub_rx: Some(calendar_rx),
            tasks_hub_rx: Some(tasks_rx),
        };
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
                        if self.inputting {
                            self.input_handle_key_event(key_event);
                        } else {
                            self.handle_key_event(key_event);
                        }
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

    fn input_handle_key_event(&mut self, key_event: KeyEvent) {
        match (key_event.modifiers, key_event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) | (_, KeyCode::Esc) => self.cancel_input(),
            (KeyModifiers::NONE, KeyCode::Char(ch)) | (KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
                self.insert_char_at(ch, self.cursor_index);
                self.cursor_index += 1;
            }
            (KeyModifiers::NONE, KeyCode::Enter) => self.update_or_create_task_or_event(),
            (KeyModifiers::NONE, KeyCode::Left) | (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
                if self.cursor_index > 0 {
                    self.cursor_index -= 1
                }
            }
            (KeyModifiers::NONE, KeyCode::Right) | (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
                if self.cursor_index < self.input_buffer.len() {
                    self.cursor_index += 1
                }
            }
            (KeyModifiers::NONE, KeyCode::Backspace)
            | (KeyModifiers::CONTROL, KeyCode::Char('h')) => {
                if self.cursor_index > 0 {
                    self.remove_char_at(self.cursor_index - 1);
                    self.cursor_index -= 1
                }
            }
            (KeyModifiers::NONE, KeyCode::Delete) | (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                if self.cursor_index < self.input_buffer.len() {
                    self.remove_char_at(self.cursor_index);
                }
            }
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => self.cursor_index = 0,
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => self.cursor_index = self.char_count(),
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                let byte_pos = self.byte_offset_at_char(self.cursor_index);
                self.input_buffer.drain(..byte_pos);
                self.cursor_index = 0
            }
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                let byte_pos = self.byte_offset_at_char(self.cursor_index);
                self.input_buffer.drain(byte_pos..);
                self.cursor_index = self.char_count()
            }
            _ => {}
        }
    }

    fn char_count(&self) -> usize {
        self.input_buffer.chars().count()
    }

    fn byte_offset_at_char(&self, char_idx: usize) -> usize {
        self.input_buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input_buffer.len())
    }

    fn remove_char_at(&mut self, char_idx: usize) {
        if char_idx >= self.char_count() {
            return;
        }
        let byte_pos = self.byte_offset_at_char(char_idx);
        let char_len = self.input_buffer.chars().nth(char_idx).unwrap().len_utf8();
        self.input_buffer.drain(byte_pos..byte_pos + char_len);
    }

    fn insert_char_at(&mut self, ch: char, char_idx: usize) {
        let byte_pos = self.byte_offset_at_char(char_idx);
        self.input_buffer.insert(byte_pos, ch);
    }

    fn cancel_input(&mut self) {
        self.input_buffer.clear();
        self.updating_event_or_task = false;
        self.cursor_index = 0;
        self.inputting = false
    }

    fn create_task_or_event(&mut self) {
        // Trimming and checking empty is already done here
        if self.input_buffer.trim().is_empty() {
            self.cancel_input();
            return;
        }

        let title = self.input_buffer.trim().to_string();
        self.input_buffer.clear();
        self.cursor_index = 0;
        self.inputting = false;

        if let AppLayout::Tasks(_) = self.app_layout {
            self.create_task_in_background(title);
        } else {
            self.create_event_in_background(title);
        }
    }

    fn update_or_create_task_or_event(&mut self) {
        // Trimming and checking empty is done here
        if self.input_buffer.trim().is_empty() {
            self.cancel_input();
            return;
        }
        if self.updating_event_or_task {
            self.updating_event_or_task = false;
            let title = self.input_buffer.trim().to_string();
            self.input_buffer.clear();
            self.inputting = false;

            if let AppLayout::Tasks(_) = self.app_layout {
                if !self.tasks_cache.is_empty() {
                    self.update_task_in_background(title);
                    return;
                }
            } else {
                if !self.events_cache.is_empty() {
                    self.update_event_in_background(title);
                    return;
                }
            }
        }
        self.create_task_or_event()
    }

    fn update_event_in_background(&mut self, title: String) {
        // Trimming and checking empty is already done
        let Some(hub) = self.event_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::Red);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone();
        self.changing_status = ("Creating event".to_string(), StatusColor::Yellow);

        // Use current_date as the day
        let date = self.current_date;
        let current_event = self.selected_event().unwrap().clone();
        let updated_event = match parse_input::parse_time_range(&title.trim(), date) {
            (title, Some(start_datetime), Some(end_datetime), _, _) => {
                let start_tz = start_datetime
                    .and_local_timezone(self.app_tz)
                    .latest()
                    .unwrap()
                    .to_utc();
                let start = api::EventDateTime {
                    date: None,
                    date_time: Some(start_tz),
                    time_zone: None,
                };
                let end_tz = end_datetime
                    .and_local_timezone(self.app_tz)
                    .latest()
                    .unwrap()
                    .to_utc();
                let end = api::EventDateTime {
                    date: None,
                    date_time: Some(end_tz),
                    time_zone: None,
                };

                api::Event {
                    summary: Some(title),
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }
            }
            (title, _, _, Some(start_date), Some(end_date)) => {
                let start = api::EventDateTime {
                    date: Some(start_date),
                    date_time: None,
                    time_zone: None,
                };
                let end = api::EventDateTime {
                    date: Some(end_date),
                    date_time: None,
                    time_zone: None,
                };
                api::Event {
                    summary: Some(title),
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }
            }
            (title, _, _, _, _) => api::Event {
                summary: Some(title),
                ..Default::default()
            },
        };

        tokio::spawn(async move {
            let result = hub
                .events()
                .patch(
                    updated_event,
                    &current_event.1,
                    &current_event.0.id.unwrap(),
                )
                .doit()
                .await;

            let msg = match result {
                Ok((_, _)) => {
                    // You could update cache with real ID here if you track it
                    ("Event created!".to_string(), StatusColor::Green)
                }
                Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn update_task_in_background(&mut self, title: String) {
        // Trimming and checking empty is already done
        let Some(hub) = self.task_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::Red);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone(); // Reuse channel or make separate
        self.changing_status = ("Updating task".to_string(), StatusColor::Yellow);

        let (updating_task, updating_tasklist_id) = self.selected_task().unwrap().clone();
        let current_year = self.current_date.year();
        let updated_task = match parse_input::parse_date_and_note(&title, current_year) {
            (t, due, notes) => Task {
                title: Some(t),
                due: due,
                notes: notes,
                ..Task::default()
            },
        };

        tokio::spawn(async move {
            let msg = {
                let result = hub
                    .tasks()
                    .patch(
                        updated_task,
                        &updating_tasklist_id,
                        &updating_task.id.unwrap(),
                    )
                    .doit()
                    .await;

                match result {
                    Ok((_, _)) => {
                        // You could update cache with real ID here if you track it
                        ("Task updated!".to_string(), StatusColor::Green)
                    }
                    Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
                }
            };
            let _ = tx.send(msg).await;
        });
    }

    fn create_task_in_background(&mut self, title: String) {
        // Trimming and checking empty is already done
        let Some(hub) = self.task_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::Red);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone(); // Reuse channel or make separate
        self.changing_status = ("Creating task".to_string(), StatusColor::Yellow);
        self.cursor_line = 0;

        let current_year = self.current_date.year();
        let new_task = match parse_input::parse_date_and_note(&title, current_year) {
            (t, due, notes) => Task {
                title: Some(t),
                due: due,
                notes: notes,
                ..Task::default()
            },
        };

        tokio::spawn(async move {
            let tasklists = match hub.tasklists().list().doit().await {
                Ok((_, tasks_list)) => tasks_list.items.unwrap_or_default(),
                Err(e) => {
                    eprintln!("Failed to fetch tasklists: {e:?}");
                    Vec::new()
                }
            };

            let msg = match tasklists.first() {
                None => ("No Tasklist!".to_string(), StatusColor::Red),
                Some(primary_tasklist) => {
                    let result = hub
                        .tasks()
                        .insert(new_task, primary_tasklist.id.as_ref().unwrap()) // Use primary list
                        .doit()
                        .await;

                    match result {
                        Ok((_, _)) => {
                            // You could update cache with real ID here if you track it
                            ("Task created!".to_string(), StatusColor::Green)
                        }
                        Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
                    }
                }
            };
            let _ = tx.send(msg).await;
        });
    }

    fn create_event_in_background(&mut self, title: String) {
        // Trimming and checking empty is already done
        let Some(hub) = self.event_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::Red);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone();
        self.changing_status = ("Creating event".to_string(), StatusColor::Yellow);

        // Use current_date as the day
        let date = self.current_date;
        let new_event = match parse_input::parse_time_range(&title.trim(), date) {
            (title, Some(start_datetime), Some(end_datetime), _, _) => {
                let start_tz = start_datetime
                    .and_local_timezone(self.app_tz)
                    .latest()
                    .unwrap()
                    .to_utc();
                let start = api::EventDateTime {
                    date: None,
                    date_time: Some(start_tz),
                    time_zone: None,
                };
                let end_tz = end_datetime
                    .and_local_timezone(self.app_tz)
                    .latest()
                    .unwrap()
                    .to_utc();
                let end = api::EventDateTime {
                    date: None,
                    date_time: Some(end_tz),
                    time_zone: None,
                };

                api::Event {
                    summary: Some(title),
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }
            }
            (title, _, _, Some(start_date), Some(end_date)) => {
                let start = api::EventDateTime {
                    date: Some(start_date),
                    date_time: None,
                    time_zone: None,
                };
                let end = api::EventDateTime {
                    date: Some(end_date),
                    date_time: None,
                    time_zone: None,
                };
                api::Event {
                    summary: Some(title),
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }
            }
            (title, _, _, _, _) => {
                let start = api::EventDateTime {
                    date: Some(date),
                    date_time: None,
                    time_zone: None,
                };
                let end = api::EventDateTime {
                    date: Some(date.succ_opt().unwrap()),
                    date_time: None,
                    time_zone: None,
                };
                api::Event {
                    summary: Some(title),
                    start: Some(start),
                    end: Some(end),
                    ..Default::default()
                }
            }
        };

        tokio::spawn(async move {
            let result = hub.events().insert(new_event, "primary").doit().await;

            let msg = match result {
                Ok((_, _)) => {
                    // You could update cache with real ID here if you track it
                    ("Event created!".to_string(), StatusColor::Green)
                }
                Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
            };
            let _ = tx.send(msg).await;
        });
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

    fn current_day_events(&self) -> &[(api::Event, String)] {
        static EMPTY: Vec<(api::Event, String)> = Vec::new();
        self.events_cache
            .get(&self.current_date)
            .map(|v| v.as_slice())
            .unwrap_or(&EMPTY)
    }

    fn selected_event_index(&self) -> Option<usize> {
        let events = self.current_day_events();
        if events.is_empty() {
            return None;
        }

        let idx = self.cursor_line;
        if idx < events.len() {
            Some(idx)
        } else {
            Some(events.len().saturating_sub(1))
        }
    }

    fn selected_task_index(&self) -> Option<usize> {
        if self.tasks_cache.is_empty() {
            return None;
        }

        let idx = self.cursor_line;
        if idx < self.tasks_cache.len() {
            Some(idx)
        } else {
            Some(self.tasks_cache.len().saturating_sub(1))
        }
    }

    fn selected_event(&self) -> Option<&(api::Event, String)> {
        let idx = self.selected_event_index()?;
        self.current_day_events().get(idx)
    }

    fn selected_task(&self) -> Option<&(Task, String)> {
        let idx = self.selected_task_index()?;
        self.tasks_cache.get(idx)
    }

    fn start_background_refresh(&mut self) {
        self.start_background_event_fetch();
        self.start_background_task_fetch();
    }

    fn start_background_event_fetch(&mut self) {
        if let Some(hub) = self.event_hub.clone() {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.events_update_rx = Some(rx);
            self.refreshing_status = ("Refreshing".to_string(), StatusColor::Green);
            let offset = self.app_tz.clone();
            tokio::spawn(async move {
                if let Some(new_events) = App::fetch_events(offset, &hub).await {
                    file_writing::save_events_cache(&new_events);
                    let _ = tx.send(new_events).await;
                }
            });
        }
    }
    fn start_background_task_fetch(&mut self) {
        if let Some(hub) = self.task_hub.clone() {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            self.tasks_update_rx = Some(rx);
            self.refreshing_status = ("Refreshing".to_string(), StatusColor::Green);
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
                self.refreshing_status = ("".to_string(), StatusColor::White);
            }
        }
        if let Some(rx) = &mut self.tasks_update_rx {
            if let Ok(new_cache) = rx.try_recv() {
                self.tasks_cache = new_cache;
                self.refreshing_status = ("".to_string(), StatusColor::White);
            }
        }

        if let Some(rx) = &mut self.change_feedback_rx {
            if let Ok(msg) = rx.try_recv() {
                self.changing_status = msg;
                self.needs_refresh = true;
            }
        }

        if let Some(rx) = &mut self.calendar_hub_rx {
            if let Ok(hub) = rx.try_recv() {
                self.event_hub = hub;
                if self.event_hub.is_some() {
                    self.start_background_event_fetch();
                }
                self.update_auth_status();
                self.calendar_hub_rx = None;
            }
        }

        if let Some(rx) = &mut self.tasks_hub_rx {
            if let Ok(hub) = rx.try_recv() {
                self.task_hub = hub;
                if self.task_hub.is_some() {
                    self.start_background_task_fetch();
                }
                self.update_auth_status();
                self.tasks_hub_rx = None;
            }
        }
    }

    fn update_auth_status(&mut self) {
        if self.event_hub.is_some() || self.task_hub.is_some() {
            self.auth_status = AuthStatus::Online;
        } else {
            self.auth_status = AuthStatus::Offline;
        }
    }

    fn delete_selected_event(&mut self) {
        let Some(event) = self.selected_event().cloned() else {
            return;
        };

        let Some(event_id) = event.0.id else {
            return;
        };

        let Some(hub) = self.event_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::White);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone();
        self.changing_status = ("Deleting".to_string(), StatusColor::Yellow);

        // Spawn background deletion
        tokio::spawn(async move {
            let result = hub.events().delete(&event.1, &event_id).doit().await;

            let msg = match result {
                Ok(_) => ("Event Deleted!".to_string(), StatusColor::Green),
                Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
            };
            let _ = tx.send(msg).await;
        });
    }

    fn delete_selected_task(&mut self) {
        let Some(task) = self.selected_task().cloned() else {
            return;
        };
        let Some(task_id) = task.0.id else {
            return;
        };
        let Some(hub) = self.task_hub.as_ref().cloned() else {
            self.changing_status = ("Offline".to_string(), StatusColor::White);
            return;
        };

        let tx = self.change_feedback_tx.as_ref().unwrap().clone();
        self.changing_status = ("Deleting task...".to_string(), StatusColor::Yellow);

        tokio::spawn(async move {
            let result = hub.tasks().delete(&task.1, &task_id).doit().await;
            let msg = match result {
                Ok(_) => ("Task deleted!".to_string(), StatusColor::Green),
                Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
            };
            let _ = tx.send(msg).await.ok();
        });
    }

    async fn fetch_events(
        app_tz: FixedOffset,
        hub: &CalendarHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>,
    ) -> Option<HashMap<NaiveDate, Vec<(api::Event, String)>>> {
        let calendars = match hub.calendar_list().list().doit().await {
            Ok((_, calendar_ids)) => calendar_ids.items.unwrap_or_default(),
            Err(e) => {
                eprintln!("Failed to fetch calendars: {e:?}");
                return None;
            }
        };

        let mut map: HashMap<NaiveDate, Vec<(api::Event, String)>> = HashMap::new();

        for entry in calendars {
            if let Some(id) = entry.id {
                let re_encoded_id = urlencoding::encode(&id);
                match hub
                    .events()
                    .list(&re_encoded_id)
                    .add_scope(google_calendar3::api::Scope::Full)
                    .single_events(true)
                    .order_by("startTime")
                    .doit()
                    .await
                {
                    Ok((_, events_list)) => {
                        if let Some(items) = events_list.items {
                            for event in items {
                                let start_date_and_event = if let Some(start) = &event.start {
                                    if let Some(date_time_str) = start.date_time {
                                        // Convert to your local timezone and get the local date + time
                                        let local_dt = date_time_str.with_timezone(&app_tz);
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
                                    map.entry(start_date)
                                        .or_default()
                                        .push((event, re_encoded_id.to_string().clone()));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to fetch events: {e:?}");
                    }
                }
            }
        }
        Some(map)
    }

    async fn fetch_tasks(
        hub: &TasksHub<hyper_rustls::HttpsConnector<connect::HttpConnector>>,
    ) -> Option<Vec<(Task, String)>> {
        let tasklists = match hub.tasklists().list().doit().await {
            Ok((_, tasks_list)) => tasks_list.items.unwrap_or_default(),
            Err(e) => {
                eprintln!("Failed to fetch tasklists: {e:?}");
                return None;
            }
        };
        let mut all_tasks = Vec::new();
        for tasklist in tasklists {
            if let Some(tasklist_id) = tasklist.id {
                match hub.tasks().list(&tasklist_id).doit().await {
                    Ok((_, tasks)) => {
                        if let Some(items) = tasks.items {
                            let tasks_with_list: Vec<(Task, String)> = items
                                .iter()
                                .map(|t| (t.clone(), tasklist_id.clone()))
                                .collect();
                            all_tasks.extend(tasks_with_list);
                        }
                    }
                    Err(e) => eprintln!("Failed to fetch tasks for list {tasklist_id}: {e:?}"),
                }
            }
        }
        Some(all_tasks)
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Esc => self.exit(),
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
            KeyCode::Char('y') => {
                self.current_date = self
                    .current_date
                    .checked_add_months(Months::new(12))
                    .unwrap()
            }
            KeyCode::Char('Y') => {
                self.current_date = self
                    .current_date
                    .checked_sub_months(Months::new(12))
                    .unwrap()
            }
            KeyCode::Char('D') => match self.app_layout {
                AppLayout::Tasks(_) => {
                    self.delete_selected_task();
                }
                AppLayout::Events => {
                    self.delete_selected_event();
                }
                _ => {}
            },
            KeyCode::Enter => match self.app_layout {
                AppLayout::Tasks(false) => {
                    self.app_layout = AppLayout::Tasks(true);
                }
                _ => {}
            },
            KeyCode::Char('E') => self.toggle_event_visibility(),
            KeyCode::Char('T') => self.toggle_tasks_visibility(),
            KeyCode::Char('t') => self.current_date = self.today,
            KeyCode::Char('R') => self.needs_refresh = true,
            KeyCode::Char('o') => self.inputting = true,
            KeyCode::Char('a') => self.add_or_update_event(),
            KeyCode::Char(' ') => self.toggle_task_completed(),
            KeyCode::Char('L') => self.clear_completed_tasks(),
            _ => {}
        }
    }

    fn toggle_task_completed(&mut self) {
        match self.app_layout {
            AppLayout::Tasks(_) => {
                let Some(task) = self.selected_task().cloned() else {
                    return;
                };
                let Some(task_id) = task.0.id else {
                    return;
                };
                let Some(hub) = self.task_hub.as_ref().cloned() else {
                    self.changing_status = ("Offline".to_string(), StatusColor::White);
                    return;
                };
                let Some(completed_status) = task.0.status else {
                    return;
                };
                let new_completed = match completed_status.as_str() {
                    "completed" => Task {
                        status: Some("needsAction".to_string()),
                        ..Default::default()
                    },
                    "needsAction" => Task {
                        status: Some("completed".to_string()),
                        ..Default::default()
                    },
                    _ => Task::default(),
                };

                let tx = self.change_feedback_tx.as_ref().unwrap().clone();
                self.changing_status = ("Toggling...".to_string(), StatusColor::Yellow);

                tokio::spawn(async move {
                    let result = hub
                        .tasks()
                        .patch(new_completed, &task.1, &task_id)
                        .doit()
                        .await;
                    let msg = match result {
                        Ok(_) => ("Completed".to_string(), StatusColor::Green),
                        Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
                    };
                    let _ = tx.send(msg).await.ok();
                });
            }
            _ => {}
        }
    }

    fn clear_completed_tasks(&mut self) {
        match self.app_layout {
            AppLayout::Tasks(_) => {
                let Some(task) = self.selected_task().cloned() else {
                    return;
                };
                let Some(hub) = self.task_hub.as_ref().cloned() else {
                    self.changing_status = ("Offline".to_string(), StatusColor::White);
                    return;
                };
                let tx = self.change_feedback_tx.as_ref().unwrap().clone();
                self.changing_status = ("Clearing...".to_string(), StatusColor::Yellow);

                tokio::spawn(async move {
                    let result = hub
                        .tasks()
                        .clear(&task.1)
                        .doit()
                        .await;
                    let msg = match result {
                        Ok(_) => ("Cleared".to_string(), StatusColor::Green),
                        Err(e) => (format!("Failed: {e}").to_string(), StatusColor::Red),
                    };
                    let _ = tx.send(msg).await.ok();
                });
            }
            _ => {}
        }
    }

    fn add_or_update_event(&mut self) {
        self.updating_event_or_task = true;
        match self.app_layout {
            AppLayout::Tasks(_) => {
                if let Some(selected_task) = self.selected_task() {
                    self.input_buffer = selected_task.0.title.as_ref().unwrap().to_string();
                    self.cursor_index = self.char_count();
                    self.inputting = true;
                    return;
                }
            }
            AppLayout::Events => {
                if let Some(selected_event) = self.selected_event() {
                    self.input_buffer = selected_event.0.summary.as_ref().unwrap().to_string();
                    self.cursor_index = self.char_count();
                    self.inputting = true;
                    return;
                }
            }
            _ => {}
        }
        // 'a' adds event when on calendar
        self.updating_event_or_task = false;
        self.inputting = true
    }

    fn exit(&mut self) {
        match self.app_layout {
            AppLayout::Events => {
                self.app_layout = AppLayout::Calendar;
            }
            AppLayout::Tasks(true) => {
                self.app_layout = AppLayout::Tasks(false);
            }
            _ => {
                self.exit = true;
            }
        }
    }

    fn move_right(&mut self) {
        match self.app_layout {
            AppLayout::Events | AppLayout::Tasks(_) => {
                return;
            }
            _ => {
                self.current_date = self.current_date.succ_opt().unwrap();
            }
        }
    }

    fn move_left(&mut self) {
        match self.app_layout {
            AppLayout::Events | AppLayout::Tasks(_) => {
                return;
            }
            _ => {
                self.current_date = self.current_date.pred_opt().unwrap();
            }
        }
    }

    fn move_up(&mut self) {
        match self.app_layout {
            AppLayout::Events | AppLayout::Tasks(_) => {
                if self.cursor_line > 0 {
                    self.cursor_line = self.cursor_line - 1;
                }
            }
            _ => {
                self.current_date = self.current_date.checked_sub_days(Days::new(7)).unwrap();
            }
        }
    }

    fn move_down(&mut self) {
        match self.app_layout {
            AppLayout::Tasks(_) => {
                if self.cursor_line < self.tasks_cache.len() - 1 {
                    self.cursor_line = self.cursor_line + 1;
                }
            }
            AppLayout::Events => {
                if self.cursor_line < self.current_day_events().len() - 1 {
                    self.cursor_line = self.cursor_line + 1;
                }
            }
            _ => {
                self.current_date = self.current_date.checked_add_days(Days::new(7)).unwrap();
            }
        }
    }

    fn toggle_event_visibility(&mut self) {
        self.app_layout = match self.app_layout {
            AppLayout::Events => AppLayout::Calendar,
            _ => AppLayout::Events,
        };
        self.cursor_line = 0;
    }
    fn toggle_tasks_visibility(&mut self) {
        self.app_layout = match self.app_layout {
            AppLayout::Tasks(_) => AppLayout::Calendar,
            _ => AppLayout::Tasks(false),
        };
        self.cursor_line = 0;
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let main_chunks = Layout::new(
            Direction::Vertical,
            [
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1),
            ],
        )
        .split(area);

        let main_area = match self.app_layout {
            AppLayout::Tasks(_) => Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([70, 30]),
            )
            .split(main_chunks[1]),
            _ => Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([100, 0]),
            )
            .split(main_chunks[1]),
        };

        // Title area
        let title_area = Layout::new(
            Direction::Horizontal,
            Constraint::from_percentages([12, 76, 12]),
        )
        .split(main_chunks[0]);

        // Title
        Paragraph::new(self.current_date.format("%Y %B").to_string())
            .centered()
            .style(Modifier::BOLD)
            .render(title_area[1], buf);

        // Refreshing status
        let status_area = title_area[0].inner(ratatui::layout::Margin {
            vertical: 0,
            horizontal: 1,
        });
        let status = Paragraph::new(self.refreshing_status.clone().0).style(Modifier::BOLD);
        match self.refreshing_status.clone().1 {
            StatusColor::Green => status.green().render(status_area, buf),
            StatusColor::Yellow => status.yellow().render(status_area, buf),
            StatusColor::Red => status.red().render(status_area, buf),
            _ => status.render(status_area, buf),
        }

        // Online status
        let auth_status = match self.auth_status {
            AuthStatus::Authenticating => "Authenticating".yellow(),
            AuthStatus::Online => "Online".green(),
            AuthStatus::Offline => "Offline".dim(),
        };

        Paragraph::new(auth_status.into_right_aligned_line()).render(
            title_area[2].inner(ratatui::layout::Margin {
                vertical: 0,
                horizontal: 1,
            }),
            buf,
        );

        // Calendar area
        let calendar_area = main_area[0];
        let (drawn_dates, number_of_rows) = self.generate_calendar_grid();
        let height = (calendar_area.height as usize) / (number_of_rows);

        let mut calendar_row_constraints = vec![Constraint::Length(height as u16); number_of_rows];
        calendar_row_constraints.insert(0, Constraint::Length(3));
        let calendar_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(calendar_row_constraints)
            .split(calendar_area);

        // Calendar Header
        let weekday_area = calendar_rows[0];
        let weekday_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1); 7])
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
                .constraints([Constraint::Fill(1); 7])
                .split(*row_chunk);

            // Draw each cell in this row
            for (col_index, cell_chunk) in horizontal_chunks.iter().enumerate() {
                let cell_border = Block::default();
                let current_cell = drawn_dates[row_index][col_index];
                let current_date = current_cell.0.day();
                let is_cursor_here = cursor_date == current_date && current_cell.1;
                let focus_on_calendar = matches!(self.app_layout, AppLayout::Calendar);
                let day = if is_cursor_here && focus_on_calendar {
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
                            let title = ev.0.summary.as_deref().unwrap_or("Untitled");
                            let time =
                                ev.0.start
                                    .as_ref()
                                    .and_then(|s| s.date_time)
                                    .map(|dt| {
                                        dt.with_timezone(&self.app_tz).format("%H:%M ").to_string()
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

        match self.app_layout {
            AppLayout::Events => {
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
                        .enumerate()
                        .map(|(i, ev)| {
                            let title = ev.0.summary.as_deref().unwrap_or("Untitled");
                            let start_time =
                                ev.0.start
                                    .as_ref()
                                    .and_then(|s| s.date_time)
                                    .map(|dt| {
                                        dt.with_timezone(&self.app_tz).format(" %H:%M ").to_string()
                                    })
                                    .unwrap_or(" ".to_string());
                            let end_time =
                                ev.0.end
                                    .as_ref()
                                    .and_then(|s| s.date_time)
                                    .map(|dt| {
                                        dt.with_timezone(&self.app_tz)
                                            .format("- %H:%M ")
                                            .to_string()
                                    })
                                    .unwrap_or("".to_string());
                            let mut item = ratatui::widgets::ListItem::new(format!(
                                "{start_time}{end_time}{title}"
                            ));
                            if Some(i) == self.selected_event_index() {
                                item = item.bg(Color::DarkGray).fg(Color::White);
                            };
                            item
                        })
                        .collect()
                };

                ratatui::widgets::List::new(items)
                    .block(Block::bordered().title("Events"))
                    .render(event_area[1], buf);
            }

            AppLayout::Tasks(notes_visible) => {
                let tasks = self.tasks_cache.clone();
                let items: Vec<Span> = {
                    tasks
                        .iter()
                        .enumerate()
                        .map(|(i, ev)| {
                            let title = ev.0.title.as_deref().unwrap_or("Untitled");
                            let time = match ev.0.due.as_deref() {
                                Some(duedate) => match DateTime::parse_from_rfc3339(duedate) {
                                    Ok(e) => e.date_naive().format("%Y/%m/%d ").to_string(),
                                    Err(_) => "".to_string(),
                                },
                                None => "".to_string(),
                            };
                            let mut item = match ev.0.completed {
                                Some(_) => Span::raw(format!("{time}{title}")).dark_gray(),
                                None => Span::raw(format!("{time}{title}")),
                            };
                            if Some(i) == self.selected_task_index() {
                                item = item.bg(Color::DarkGray).fg(Color::White);
                            };
                            item
                        })
                        .collect()
                };

                ratatui::widgets::List::new(items)
                    .block(Block::bordered().title("Tasks".bold().into_centered_line()))
                    .render(
                        main_area[1].inner(ratatui::layout::Margin {
                            vertical: 1,
                            horizontal: 5,
                        }),
                        buf,
                    );

                if notes_visible && let Some(selected_task) = self.selected_task() {
                    let task_area_horizontal = Layout::new(
                        Direction::Vertical,
                        Constraint::from_percentages([16, 68, 16]),
                    )
                    .split(main_area[0]);
                    let task_area = Layout::new(
                        Direction::Horizontal,
                        Constraint::from_percentages([20, 60, 20]),
                    )
                    .split(task_area_horizontal[1]);
                    Clear::default().render(task_area[1], buf);

                    let task_notes = selected_task.0.notes.clone().unwrap_or("".to_string());

                    let task_title = selected_task.0.title.clone().unwrap_or("".to_string());

                    Paragraph::new(task_notes)
                        .wrap(ratatui::widgets::Wrap { trim: true })
                        .block(Block::bordered().title(task_title))
                        .render(task_area[1], buf);
                };
            }
            _ => {}
        }
        // Bottom Area

        let bottom_area = Layout::new(
            Direction::Horizontal,
            [
                Constraint::Length(8),
                Constraint::Fill(1),
                Constraint::Fill(1),
            ],
        )
        .split(main_chunks[2]);

        // Changing status
        let status = Paragraph::new(self.changing_status.clone().0)
            .alignment(ratatui::layout::Alignment::Right)
            .style(Modifier::BOLD);
        let status_area = bottom_area[2].inner(ratatui::layout::Margin {
            vertical: 0,
            horizontal: 1,
        });

        match self.changing_status.clone().1 {
            StatusColor::Green => status.green().render(status_area, buf),
            StatusColor::Yellow => status.yellow().render(status_area, buf),
            StatusColor::Red => status.red().render(status_area, buf),
            _ => status.render(status_area, buf),
        }

        // Text input area

        if self.inputting {
            if let AppLayout::Tasks(_) = self.app_layout {
                Paragraph::new(" Tasks: ").render(bottom_area[0], buf)
            } else {
                Paragraph::new(" Event: ").render(bottom_area[0], buf)
            }

            let char_at_cursor = if let Some(ch) = self.input_buffer.chars().nth(self.cursor_index)
            {
                Span::raw(ch.to_string()).on_white().black()
            } else {
                Span::raw("█".to_string())
            };
            let left: String = self.input_buffer.chars().take(self.cursor_index).collect();
            let right: String = self
                .input_buffer
                .chars()
                .skip(self.cursor_index + 1)
                .collect();

            let input_left = Span::raw(left);
            let input_right = Span::raw(right);
            ratatui::text::Line::from(vec![input_left, char_at_cursor, input_right])
                .render(bottom_area[1], buf)
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
