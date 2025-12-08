use google_calendar3::api::Event;
use google_calendar3::CalendarHub;
use hyper;
use hyper_rustls;
use std::fs;
use std::io::{BufReader, Write};
use std::path::Path;
use yup_oauth2::{read_application_secret, AccessToken, Authenticator, DiskTokenStorage, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

pub struct GoogleCalendar {
    hub: CalendarHub<hyper_rustls::HttpsConnector<hyper::client::HttpConnector>>,
}

impl GoogleCalendar {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // Get client secret from credentials.json
        let secret = read_application_secret("credentials.json")
            .await
            .expect("credentials.json not found. Please download from Google Cloud Console");
        
        // Create authenticator
        let auth = InstalledFlowAuthenticator::builder(
            secret,
            InstalledFlowReturnMethod::Interactive,
        )
        .persist_tokens_to_disk("token.json")
        .build()
        .await?;
        
        // Create Calendar Hub
        let hub = CalendarHub::new(
            hyper::Client::builder().build(hyper_rustls::HttpsConnector::with_native_roots()),
            auth,
        );
        
        Ok(GoogleCalendar { hub })
    }
    
    pub async fn list_calendars(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let result = self.hub.calendar_list().list().doit().await?;
        let calendars: Vec<String> = result.1
            .items
            .unwrap_or_default()
            .into_iter()
            .filter_map(|cal| cal.summary)
            .collect();
        
        Ok(calendars)
    }
    
    pub async fn list_events(
        &self,
        calendar_id: &str,
        time_min: &str,
        time_max: &str,
    ) -> Result<Vec<Event>, Box<dyn std::error::Error>> {
        let result = self.hub
            .events()
            .list(calendar_id)
            .time_min(time_min)
            .time_max(time_max)
            .single_events(true)
            .order_by("startTime")
            .doit()
            .await?;
        
        Ok(result.1.items.unwrap_or_default())
    }
    
    pub async fn get_primary_calendar_events(
        &self,
        year: i32,
        month: u32,
    ) -> Result<Vec<Event>, Box<dyn std::error::Error>> {
        // Calculate start and end of month
        use chrono::{NaiveDate, Datelike};
        
        let start_date = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let end_date = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
        };
        
        // Format for Google Calendar API (RFC3339)
        let time_min = format!("{}T00:00:00Z", start_date.format("%Y-%m-%d"));
        let time_max = format!("{}T00:00:00Z", end_date.format("%Y-%m-%d"));
        
        // Get events from primary calendar
        self.list_events("primary", &time_min, &time_max).await
    }
}
