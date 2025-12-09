use dirs::home_dir;
use google_calendar3::{CalendarHub, yup_oauth2};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::{client::legacy::Client, client::legacy::connect, rt::TokioExecutor};
use std::error::Error;

pub async fn get_calendar_hub()
-> Result<CalendarHub<HttpsConnector<connect::HttpConnector>>, Box<dyn Error>> {
    let secret_path = home_dir()
        .expect("Could not find home directory")
        .join(".cache/calpersonal/clientsecret.json");

    let token_path = home_dir()
        .expect("Could not find home directory")
        .join(".cache/calpersonal/tokencache.json");

    // 1. Load client secret (put clientsecret.json in project root)
    let secret: yup_oauth2::ApplicationSecret = yup_oauth2::read_application_secret(secret_path)
        .await
        .map_err(|e| format!("clientsecret.json not found: {}", e))?;

    // 2. Authenticator (opens browser first time, reuses tokencache.json)
    let auth = yup_oauth2::InstalledFlowAuthenticator::builder(
        secret,
        yup_oauth2::InstalledFlowReturnMethod::HTTPRedirect,
    )
    .persist_tokens_to_disk(token_path)
    .build()
    .await
    .map_err(|e| format!("Failed to create authenticator: {}", e))?;

    // 3. Correct scope for reading calendars
    let scopes = &["https://www.googleapis.com/auth/calendar.readonly"];
    auth.token(scopes)
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let https = HttpsConnectorBuilder::new()
        .with_native_roots()
        .map_err(|e| format!("Failed to load native roots: {}", e))?
        .https_or_http()
        .enable_http1()
        .build();

    let client = Client::builder(TokioExecutor::new()).build(https);
    // 6. Create and return the hub (generics inferred)
    Ok(CalendarHub::new(client, auth))
}

pub async fn test_connection(hub: &CalendarHub<HttpsConnector<connect::HttpConnector>>) -> bool {
    match hub.calendars().get("primary").doit().await {
        Ok((_, calendar)) => {
            println!(
                "YES! Connected as: {}",
                calendar.summary.as_deref().unwrap_or("Unknown")
            );
            true
        }
        Err(e) => {
            eprintln!("Not connected! Error: {e:?}");
            false
        }
    }
}
