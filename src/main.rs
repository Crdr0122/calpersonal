use chrono::{Datelike, Days, Local, Months, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::Rect,
    layout::{Constraint, Direction, Layout},
    prelude::Stylize,
    style::{Color, Modifier, Style},
    symbols,
    text::Text,
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::io;
use std::iter;

// Struct to hold our application state
struct App {
    cursor_x: usize,
    cursor_y: usize,
    tasks_visible: bool,
    events_visible: bool,
    current_date: NaiveDate, // The date being displayed
    today: NaiveDate,        // Today's date for comparison
    exit: bool,
}

impl App {
    fn new() -> App {
        let today = Local::now().date_naive();
        App {
            cursor_x: 0,
            cursor_y: 1,
            current_date: today,
            today: today,
            tasks_visible: false,
            events_visible: false,
            exit: false,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    pub fn title(&self) -> String {
        self.current_date.format("%D %B %Y").to_string()
    }

    fn first_day_of_month(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.current_date.year(), self.current_date.month(), 1).unwrap()
    }

    fn draw(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Char('E') => self.toggle_event_visibility(),
            KeyCode::Char('T') => self.toggle_tasks_visibility(),
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn move_right(&mut self) {
        if self.cursor_x < 6 {
            self.cursor_x += 1;
        } else {
            self.cursor_x = 0;
            if self.cursor_y < 4 {
                self.cursor_y += 1;
            }
        }
        self.current_date = self.current_date.succ_opt().unwrap();
    }

    fn move_left(&mut self) {
        if self.cursor_x > 0 {
            self.cursor_x -= 1;
        } else {
            self.cursor_x = 6;
            if self.cursor_y > 0 {
                self.cursor_y -= 1;
            }
        }
        self.current_date = self.current_date.pred_opt().unwrap();
    }

    fn move_up(&mut self) {
        if self.cursor_y > 0 {
            self.cursor_y -= 1;
        } else {
            self.cursor_y = 4;
        }
        self.current_date = self.current_date.checked_sub_days(Days::new(7)).unwrap();
    }

    fn move_down(&mut self) {
        if self.cursor_y < 4 {
            self.cursor_y += 1;
        } else {
            self.cursor_y = 0;
        }
        self.current_date = self.current_date.checked_add_days(Days::new(7)).unwrap();
    }

    fn toggle_event_visibility(&mut self) {
        self.events_visible = !self.events_visible;
    }
    fn toggle_tasks_visibility(&mut self) {
        self.tasks_visible = !self.tasks_visible;
    }
}

fn main() -> Result<(), io::Error> {
    let mut terminal = ratatui::init();
    let res = App::new().run(&mut terminal);
    ratatui::restore();
    res
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
            [Constraint::Percentage(3), Constraint::Fill(1)],
        )
        .split(main_area[0]);

        // Title area
        Paragraph::new(self.title())
            .centered()
            .style(Modifier::BOLD)
            .render(main_chunks[0], buf);

        // Calendar area
        let calendar_area = main_chunks[1];
        let calendar_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(Constraint::from_percentages([8, 18, 18, 18, 18, 18]))
            .split(calendar_area);

        // Calendar Header
        let weekday_area = calendar_rows[0];
        let weekday_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(Constraint::from_ratios(iter::repeat_n((1, 7), 7)))
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
        for (row_index, row_chunk) in calendar_rows[1..6].iter().enumerate() {
            let horizontal_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(Constraint::from_ratios(iter::repeat_n((1, 7), 7)))
                .split(*row_chunk);

            // Draw each cell in this row
            for (col_index, cell_chunk) in horizontal_chunks.iter().enumerate() {
                let cell_border = Block::default();
                let is_cursor_here = row_index == self.cursor_y && col_index == self.cursor_x;
                let day_number = 3;
                let day = if is_cursor_here {
                    Text::raw(format!("{}{:<30}", day_number, " ")).on_dark_gray()
                } else {
                    Text::raw(format!("{}", day_number))
                };

                if col_index == 0 {
                    // Sunday
                    let day = day.red();
                    // let name = Text::styled(day, Style::default().fg(Color::Red));
                    let cell = Paragraph::new(day);
                    let day_block = cell_border.borders(Borders::BOTTOM | Borders::LEFT);
                    let day_block = if row_index == 4 {
                        day_block
                    } else {
                        day_block.border_set(left_bottom_border)
                    };
                    cell.block(day_block).render(*cell_chunk, buf)
                } else if col_index == 6 {
                    // Saturday
                    let day = day.blue();
                    let cell = Paragraph::new(day);
                    let day_block =
                        cell_border.borders(Borders::BOTTOM | Borders::RIGHT | Borders::LEFT);
                    let day_block = if row_index == 4 {
                        day_block.border_set(left_border)
                    } else {
                        day_block.border_set(right_bottom_border)
                    };
                    cell.block(day_block).render(*cell_chunk, buf)
                } else {
                    // Weekdays
                    let cell = Paragraph::new(day);
                    let day_block = cell_border.borders(Borders::BOTTOM | Borders::LEFT);

                    let day_block = if row_index == 4 {
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
                Constraint::from_percentages([18, 64, 18]),
            )
            .split(main_area[0]);
            let event_area = Layout::new(
                Direction::Horizontal,
                Constraint::from_percentages([30, 40, 30]),
            )
            .split(event_area_horizontal[1]);
            Clear::default().render(event_area[1], buf);
            Block::bordered()
                .title("Events".bold().into_centered_line())
                .render(event_area[1], buf);
        }

        if self.tasks_visible {
            let task_area = Layout::new(
                Direction::Vertical,
                Constraint::from_percentages([2, 96, 2]),
            )
            .margin(4)
            .split(main_area[1]);
            // Clear::default().render(event_area[1], buf);
            Block::bordered()
                .title("Tasks".bold().into_centered_line())
                .render(task_area[1], buf);
        }
    }
}
