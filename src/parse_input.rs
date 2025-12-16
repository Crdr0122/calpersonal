use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};

pub fn parse_time_range(
    input: &str,
    current_date: NaiveDate,
) -> (
    String,
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
    Option<NaiveDate>,
    Option<NaiveDate>,
) {
    // Trimming and checking empty is already done

    let time_re = regex::Regex::new(r"^(\d{1,2}:\d{2})\s+-\s+(\d{1,2}:\d{2})\s").unwrap();
    let date_time_re =
        regex::Regex::new(r"^(\d{1,2}/\d{1,2})\s+(\d{1,2}:\d{2})\s+-\s+(\d{1,2}:\d{2})\s").unwrap();
    let year_date_time_re =
        regex::Regex::new(r"^(\d{4}/\d{1,2}/\d{1,2})\s+(\d{1,2}:\d{2})\s+-\s+(\d{1,2}:\d{2})\s")
            .unwrap();
    let date_re = regex::Regex::new(r"^(\d{1,2}/\d{1,2})\s+-\s+(\d{1,2}/\d{1,2})\s").unwrap();
    let year_date_re =
        regex::Regex::new(r"^(\d{4}/\d{1,2}/\d{1,2})\s+-\s+(\d{4}/\d{1,2}/\d{1,2})\s").unwrap();
    let only_date_re = regex::Regex::new(r"^(\d{1,2}/\d{1,2})\s").unwrap();
    let only_year_date_re = regex::Regex::new(r"^(\d{4}/\d{1,2}/\d{1,2})\s").unwrap();

    if let Some(caps) = time_re.captures(input) {
        let start_str = caps.get(1).unwrap().as_str();
        let end_str = caps.get(2).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveTime::parse_from_str(start_str, "%H:%M"),
            NaiveTime::parse_from_str(end_str, "%H:%M"),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (
                summary,
                Some(NaiveDateTime::new(current_date, start)),
                Some(NaiveDateTime::new(current_date, end)),
                None,
                None,
            );
        }
    } else if let Some(caps) = date_time_re.captures(input) {
        let current_year = current_date.year().to_string();
        let event_date = caps.get(1).unwrap().as_str().to_owned();
        let start_str = caps.get(2).unwrap().as_str();
        let end_str = caps.get(3).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveDateTime::parse_from_str(
                &(current_year.clone() + "/" + &event_date + ":" + start_str),
                "%Y/%m/%d:%H:%M",
            ),
            NaiveDateTime::parse_from_str(
                &(current_year + "/" + &event_date + ":" + end_str),
                "%Y/%m/%d:%H:%M",
            ),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (summary, Some(start), Some(end), None, None);
        }
    } else if let Some(caps) = year_date_time_re.captures(input) {
        let event_date = caps.get(1).unwrap().as_str().to_owned();
        let start_str = caps.get(2).unwrap().as_str();
        let end_str = caps.get(3).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveDateTime::parse_from_str(
                &(event_date.clone() + ":" + start_str),
                "%Y/%-m/%-d:%H:%M",
            ),
            NaiveDateTime::parse_from_str(&(event_date + ":" + end_str), "%Y/%-m/%-d:%H:%M"),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (summary, Some(start), Some(end), None, None);
        }
    } else if let Some(caps) = date_re.captures(input) {
        let current_year = current_date.year().to_string();
        let start_str = caps.get(1).unwrap().as_str();
        let end_str = caps.get(2).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveDate::parse_from_str(&(current_year.clone() + start_str), "%Y%-m/%-d"),
            NaiveDate::parse_from_str(&(current_year + end_str), "%Y%-m/%-d"),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (summary, None, None, Some(start), Some(end));
        }
    } else if let Some(caps) = year_date_re.captures(input) {
        let start_str = caps.get(1).unwrap().as_str();
        let end_str = caps.get(2).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveDate::parse_from_str(&(start_str), "%Y/%-m/%-d"),
            NaiveDate::parse_from_str(&(end_str), "%Y/%-m/%-d"),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (summary, None, None, Some(start), Some(end));
        }
    } else if let Some(caps) = only_date_re.captures(input) {
        let current_year = current_date.year().to_string();
        let start_str = caps.get(1).unwrap().as_str();

        if let Ok(start) =
            NaiveDate::parse_from_str(&(current_year + "/" + start_str), "%Y/%-m/%-d")
        {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (
                summary,
                None,
                None,
                Some(start),
                Some(start.succ_opt().unwrap()),
            );
        }
    } else if let Some(caps) = only_year_date_re.captures(input) {
        let start_str = caps.get(1).unwrap().as_str();

        if let Ok(start) = NaiveDate::parse_from_str(&(start_str), "%Y/%-m/%-d") {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (
                summary,
                None,
                None,
                Some(start),
                Some(start.succ_opt().unwrap()),
            );
        }
    }

    (input.to_string(), None, None, None, None)
}

pub fn parse_date_and_note(
    input: &str,
    current_year: i32,
) -> (String, Option<String>, Option<String>) {
    let mm_dd_re = regex::Regex::new(r"^(\d{1,2}/\d{1,2})\s").unwrap();
    let yyyy_mm_dd_re = regex::Regex::new(r"^(\d{4}/\d{1,2}/\d{1,2})\s").unwrap();
    // 2026-01-20T00:00:00.000Z
    let (title_without_date, due_date) = if let Some(caps) = mm_dd_re.captures(input) {
        let due_str = caps.get(1).unwrap().as_str();
        if let Ok(due) =
            NaiveDate::parse_from_str(&(current_year.to_string() + due_str), "%Y%-m/%-d")
        {
            let title_start = caps.get(0).unwrap().end();
            let title = input[title_start..].trim().to_string();
            (
                title,
                Some(due.format("%Y-%m-%dT00:00:00.000Z").to_string()),
            )
        } else {
            (input.to_string(), None)
        }
    } else if let Some(caps) = yyyy_mm_dd_re.captures(input) {
        let due_str = caps.get(1).unwrap().as_str();
        if let Ok(due) = NaiveDate::parse_from_str(due_str, "%Y/%-m/%-d") {
            let title_start = caps.get(0).unwrap().end();
            let title = input[title_start..].trim().to_string();
            (
                title,
                Some(due.format("%Y-%m-%dT00:00:00.000Z").to_string()),
            )
        } else {
            (input.to_string(), None)
        }
    } else {
        (input.to_string(), None)
    };

    let notes_re = regex::Regex::new(r"\snotes:\s(.+)$").unwrap();
    let (rem, notes) = if let Some(caps) = notes_re.captures(&title_without_date) {
        let notes_str = caps.get(1).unwrap().as_str();
        let title_end = caps.get(0).unwrap().start();
        let title = title_without_date[..title_end].trim().to_string();
        (title, Some(notes_str.to_string()))
    } else {
        (title_without_date, None)
    };

    (rem, due_date, notes)
}
