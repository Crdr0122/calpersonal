use dirs::home_dir;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub api_key: String,
    pub city: String,
    pub country: String,
}

pub fn parse_config() -> Option<Config> {
    let config_str = std::fs::read_to_string(
        home_dir()
            .expect("Could not find home directory")
            .join(".config/calpersonal/config.toml"),
    )
    .ok()?;
    toml::from_str(&config_str).expect("Config parse failed")
}
