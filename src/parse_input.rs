use chrono::{NaiveDate, NaiveTime};

pub fn parse_time_range(input: &str) -> (String, Option<NaiveTime>, Option<NaiveTime>) {
    // Trimming and checking empty is already done

    let time_re = regex::Regex::new(r"^(\d{1,2}:\d{2})\s+-\s+(\d{1,2}:\d{2})\s").unwrap();
    // let single_time_re = regex::Regex::new(r"^(\d{1,2}:\d{2})\s").unwrap();

    if let Some(caps) = time_re.captures(input) {
        let start_str = caps.get(1).unwrap().as_str();
        let end_str = caps.get(2).unwrap().as_str();

        if let (Ok(start), Ok(end)) = (
            NaiveTime::parse_from_str(start_str, "%H:%M"),
            NaiveTime::parse_from_str(end_str, "%H:%M"),
        ) {
            let summary_start = caps.get(0).unwrap().end();
            let summary = input[summary_start..].trim().to_string();
            return (summary, Some(start), Some(end));
        }
    }
    (input.to_string(), None, None)
}

pub fn parse_date(input: &str, current_year: i32) -> (String, Option<String>) {
    let mm_dd_re = regex::Regex::new(r"^(\d{1,2}/\d{1,2})\s").unwrap();
    let yyyy_mm_dd_re = regex::Regex::new(r"^(\d{4}/\d{1,2}/\d{1,2})\s").unwrap();
    // 2026-01-20T00:00:00.000Z
    if let Some(caps) = mm_dd_re.captures(input) {
        let due_str = caps.get(1).unwrap().as_str();
        if let Ok(due) =
            NaiveDate::parse_from_str(&(current_year.to_string() + due_str), "%Y%-m/%-d")
        {
            let title_start = caps.get(0).unwrap().end();
            let title = input[title_start..].trim().to_string();
            return (
                title,
                Some(due.format("%Y-%m-%dT00:00:00.000Z").to_string()),
            );
        }
    } else if let Some(caps) = yyyy_mm_dd_re.captures(input) {
        let due_str = caps.get(1).unwrap().as_str();
        if let Ok(due) = NaiveDate::parse_from_str(due_str, "%Y/%-m/%-d") {
            let title_start = caps.get(0).unwrap().end();
            let title = input[title_start..].trim().to_string();
            return (
                title,
                Some(due.format("%Y-%m-%dT00:00:00.000Z").to_string()),
            );
        }
    }
    (input.to_string(), None)
}
