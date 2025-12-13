use chrono::NaiveDate;
use dirs::home_dir;
use google_calendar3::api;
use std::collections::HashMap;
use std::fs::{read_to_string, write};

const EVENTS_CACHE_FILE: &str = "/.cache/calpersonal/calendar_cache/events_cache.json";
const TASKS_CACHE_FILE: &str = "/.cache/calpersonal/task_cache/tasks_cache.json";

pub fn load_events_cache() -> HashMap<NaiveDate, Vec<api::Event>> {
    let secret_path = home_dir()
        .expect("Could not find home directory")
        .join(EVENTS_CACHE_FILE);

    match read_to_string(secret_path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(), // Deserialize or default on error
        Err(_) => HashMap::new(),                                    // File missing → empty cache
    }
}

pub fn save_events_cache(cache: &HashMap<NaiveDate, Vec<api::Event>>) {
    let secret_path = home_dir()
        .expect("Could not find home directory")
        .join(EVENTS_CACHE_FILE);
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = write(secret_path, json); // Ignore write errors (e.g., permissions)
    }
}

pub fn load_tasks_cache() -> Vec<google_tasks1::api::Task> {
    let secret_path = home_dir()
        .expect("Could not find home directory")
        .join(TASKS_CACHE_FILE);
    match read_to_string(secret_path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_tasks_cache(cache: &[google_tasks1::api::Task]) {
    let secret_path = home_dir()
        .expect("Could not find home directory")
        .join(TASKS_CACHE_FILE);
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = write(secret_path, json);
    }
}
