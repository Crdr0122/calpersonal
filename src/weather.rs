use reqwest;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct OneCallResponse {
    pub current: WeatherData,
    pub daily: Vec<DailyWeather>,
    pub alerts: Vec<Alert>,
}
#[derive(Deserialize)]
pub struct WeatherData {
    pub temp: f64,
    pub feels_like: f64,
    pub humidity: f64,
    pub wind_speed: f64,
    pub pressure: f64,
    pub uvi: f64,
    pub clouds: f64,
    pub rain: Option<Rain>,
    pub snow: Option<Snow>,
    pub weather: Vec<Weather>,
}
#[derive(Deserialize)]
pub struct Weather {
    pub main: String,
    pub icon: String,
}
#[derive(Deserialize)]
pub struct Rain {
    #[serde(rename = "1h")]
    pub one_hour: Option<f64>,
}
#[derive(Deserialize)]
pub struct Snow {
    #[serde(rename = "1h")]
    pub one_hour: Option<f64>,
}
#[derive(Deserialize)]
pub struct Geocode {
    lat: f64,
    lon: f64,
}

#[derive(Deserialize)]
pub struct DailyWeather {
    pub temp: DailyTemp,
    pub humidity: f64,
    pub wind_speed: f64,
    pub pressure: f64,
    pub uvi: f64,
    pub rain: Option<f64>,
    pub snow: Option<f64>,
    pub weather: Vec<Weather>,
    pub pop: f64,
}
#[derive(Deserialize)]
pub struct DailyTemp {
    pub max: f64,
    pub min: f64,
}

#[derive(Deserialize)]
pub struct Alert {
    pub sender_name: String,
    pub event: String,
    pub description: String,
}

pub async fn fetch_weather(
    api_key: &str,
    city: String,
    country: String,
) -> Option<OneCallResponse> {
    let geo_url = format!(
        "http://api.openweathermap.org/geo/1.0/direct?q={},{}&limit=1&appid={}",
        city, country, api_key
    );
    if let Some(geocode_response) = reqwest::get(&geo_url).await.ok() {
        if geocode_response.status().is_success() {
            let geo: Vec<Geocode> = geocode_response.json().await.ok()?;
            let onecall_url = format!(
                "https://api.openweathermap.org/data/3.0/onecall?lat={}&lon={}&exclude=minutely,hourly&units=metric&appid={}",
                geo[0].lat, geo[0].lon, api_key
            );
            if let Some(onecall_response) = reqwest::get(&onecall_url).await.ok() {
                if onecall_response.status().is_success() {
                    let res: OneCallResponse =
                        onecall_response.json().await.expect("Could not decode");
                    return Some(res);
                }
            }
        }
    }
    None
}

pub fn get_weather_icon(icon: String) -> String {
    match icon.as_str() {
        "01d" | "01n" => r"
   \   /  
    .-.    
 ― (   ) ―
    `-’
   /   \ 
"
        .to_string(),
        "02d" | "02n" => r#"
  \  /      
_ /"".-.    
  \_(   ).  
  /(___(__) 
 
"#
        .to_string(),
        "03d" | "03n" | "04d" | "04n" => r"

   .-.   
  (   ). 
 (___(__)
 
"
        .to_string(),
        "09d" | "09n" => r"
   .-.   
  (   ). 
 (___(__)
  ‘ ‘ ‘ ‘
 ‘ ‘ ‘ ‘
"
        .to_string(),
        "10d" | "10n" => r"
   .-.   
  (   ). 
 (___(__)
 ,‘,‘,‘,‘
 ,’,’,’,’
"
        .to_string(),
        "11d" | "11n" => r"
   .-.	
  ( _ ).  
 (_./ /_)
 ‘ ‘/_ ,'
 ,‘,‘/'‘,
"
        .to_string(),
        "13d" | "13n" => r"
   .-.   
  (   ). 
 (___(__)
  *  *  *
 *  *  *
"
        .to_string(),
        "50d" | "50n" => r"

_ - _ - _ 
 _ - _ - _ 
_ - _ - _ 

"
        .to_string(),
        _ => "".to_string(),
    }
}
