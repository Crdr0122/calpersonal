use openweathermap_client::models::{City, CurrentWeather, UnitSystem};
use openweathermap_client::{Client, ClientOptions};

pub async fn get_weather(api_key: String, city: String, country: String) -> Option<CurrentWeather> {
    let options = ClientOptions {
        api_key: api_key,
        language: "en".to_string(),
        units: UnitSystem::Metric,
    };
    let client = Client::new(options).ok()?;
    let result = client
        .fetch_weather(&City::new(&city, &country))
        .await
        .ok()?;
    Some(result)
}

pub fn get_weather_icon(icon: String) -> String {
    match icon.as_str() {
        "01d" | "01n" => r"
   \   /  
    .-.    
 ― (   ) ―
    `-’
   /   \ 
".to_string(),
        "02d" | "02n" => r#"
  \  /      
_ /"".-.    
  \_(   ).  
  /(___(__) 
 
"#.to_string(),
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
".to_string(),
        "11d" | "11n" => r"
   .-.   
  (   ). 
 (___(__)
  *  *  *
 *  *  *
".to_string(),
        "13d" | "13n" => "󰖘".to_string(),
        "50d" | "50n" => r"

_ - _ - _ 
 _ - _ - _ 
_ - _ - _ 

".to_string(),
        _ => "".to_string(),
    }
}
