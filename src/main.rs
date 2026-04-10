use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use colored::*;
use reqwest;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};
use unicode_width::UnicodeWidthStr;

// --- Geocoding API Structures ---
#[derive(Debug, Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(Debug, Deserialize, Clone)]
struct GeocodingResult {
    latitude: f64,
    longitude: f64,
    name: String,
    country: String,
}

// --- Weather API Structures ---
#[derive(Debug, Deserialize)]
struct WeatherResponse {
    hourly: HourlyData,
    daily: DailyData,
    #[allow(dead_code)]
    hourly_units: HourlyUnits,
    #[allow(dead_code)]
    daily_units: DailyUnits,
}

#[derive(Debug, Deserialize)]
struct HourlyData {
    time: Vec<String>,
    temperature_2m: Vec<f64>,
    apparent_temperature: Vec<f64>,
    weathercode: Vec<u8>,
    precipitation_probability: Vec<Option<u8>>,
    windspeed_10m: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct DailyData {
    time: Vec<String>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    apparent_temperature_max: Vec<f64>,
    #[allow(dead_code)]
    apparent_temperature_min: Vec<f64>,
    weathercode: Vec<u8>,
    precipitation_probability_max: Vec<Option<u8>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HourlyUnits {
    temperature_2m: String,
    apparent_temperature: String,
    windspeed_10m: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DailyUnits {
    temperature_2m_max: String,
    temperature_2m_min: String,
    apparent_temperature_max: String,
    apparent_temperature_min: String,
}

// --- Simplified Weather Text (easy to understand) ---
fn get_weather_text(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Clear",
        2 => "Cloudy",
        3 => "Cloudy",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Light Rain",
        56 | 57 => "Freezing Rain",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing Rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow",
        80 | 81 | 82 => "Rain",
        85 | 86 => "Snow",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm",
        _ => "Unknown",
    }
}

// --- Weather Colours: cloudy bright white, fog normal white ---
fn get_weather_colour(code: u8) -> Color {
    match code {
        0 | 1 => Color::Yellow,                 // Clear sky
        2 | 3 => Color::BrightWhite,            // Cloudy – bright white
        45 | 48 => Color::White,                // Fog – normal white
        51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => Color::Blue, // Rain
        71 | 73 | 75 | 77 | 85 | 86 => Color::BrightCyan, // Snow
        95 | 96 | 99 => Color::Magenta,         // Thunderstorm
        _ => Color::White,
    }
}

// --- Temperature Bar with 9 blocks, 3 green, 3 yellow, 3 red ---
fn draw_temp_bar_compact(temp: f64, min: f64, max: f64) -> String {
    let range = max - min;
    if range <= 0.0 {
        return "▰▰▰▰▰▰▰▰▰".to_string();
    }
    let filled = ((temp - min) / range * 9.0).round() as usize;
    let filled = filled.clamp(0, 9);

    let mut result = String::new();
    for i in 0..9 {
        let block = if i < filled { "▰" } else { "▱" };
        let colored = if i < filled {
            if i < 3 {
                block.green()
            } else if i < 6 {
                block.yellow()
            } else {
                block.red()
            }
        } else {
            block.white().dimmed()
        };
        result.push_str(&colored.to_string());
    }
    result
}

// --- Pad single‑digit numbers with a leading space for alignment ---
fn pad_number(s: &str) -> String {
    let mut split_idx = 0;
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_digit() || c == '.' || c == '-' {
            continue;
        } else {
            split_idx = i;
            break;
        }
    }
    if split_idx == 0 {
        return s.to_string();
    }
    let num_str = &s[0..split_idx];
    let suffix = &s[split_idx..];
    let integer_part = num_str.split('.').next().unwrap_or(num_str);
    if integer_part.len() == 1 && !integer_part.starts_with('-') {
        format!(" {}{}", num_str, suffix)
    } else {
        s.to_string()
    }
}

// --- Formatting helpers ---
fn format_temp(value: f64, color: Color) -> String {
    let plain = format!("{:.1}°C", value);
    let padded = pad_number(&plain);
    padded.color(color).to_string()
}

fn format_feels(value: f64) -> String {
    let plain = format!("{:.1}°C", value);
    let padded = pad_number(&plain);
    padded.cyan().to_string()
}

// Rain formatting with fixed width (3 characters) for central alignment
fn format_rain(percent: u8) -> String {
    let plain = if percent < 10 {
        format!(" {}%", percent)
    } else {
        format!("{}%", percent)
    };
    if percent > 0 {
        plain.blue().to_string()
    } else {
        plain.dimmed().to_string()
    }
}

fn format_wind(speed: f64) -> String {
    let plain = format!("{:.0} km/h", speed);
    pad_number(&plain)
}

// --- Compute visible width (strips ANSI) ---
fn visible_width(s: &str) -> usize {
    let mut clean = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape && ch == 'm' {
            in_escape = false;
        } else if !in_escape {
            clean.push(ch);
        }
    }
    UnicodeWidthStr::width(clean.as_str())
}

// --- Center a string within a given visible width ---
fn center_to_width(s: &str, target_width: usize) -> String {
    let current = visible_width(s);
    if current >= target_width {
        s.to_string()
    } else {
        let left = (target_width - current) / 2;
        let right = target_width - current - left;
        format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
    }
}

// --- Parse ISO time string to local DateTime ---
fn parse_time_to_local(s: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
                .map(|naive| Utc.from_utc_datetime(&naive).with_timezone(&Local))
        })
        .unwrap_or_else(|e| panic!("Failed to parse time '{}': {}", s, e))
}

// --- Group hourly data by date ---
fn group_hourly_by_day(
    times: &[String],
    temps: &[f64],
    feels: &[f64],
    codes: &[u8],
    precip: &[Option<u8>],
    wind: &[f64],
) -> HashMap<String, (Vec<String>, Vec<f64>, Vec<f64>, Vec<u8>, Vec<u8>, Vec<f64>)> {
    let mut map: HashMap<
        String,
        (Vec<String>, Vec<f64>, Vec<f64>, Vec<u8>, Vec<u8>, Vec<f64>),
    > = HashMap::new();
    for i in 0..times.len() {
        let dt = parse_time_to_local(&times[i]);
        let date_key = dt.format("%Y-%m-%d").to_string();
        let entry = map.entry(date_key).or_insert((
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        entry.0.push(dt.format("%H").to_string());
        entry.1.push(temps[i]);
        entry.2.push(feels[i]);
        entry.3.push(codes[i]);
        entry.4.push(precip[i].unwrap_or(0));
        entry.5.push(wind[i]);
    }
    map
}

// --- Print Hourly Table (all columns centered) ---
fn print_hourly_table(rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }

    let headers = ["Hr", "Weather", "Temperature", "Feel", "Rain", "Wind"];
    let mut col_widths = vec![0; headers.len()];

    for (i, header) in headers.iter().enumerate() {
        col_widths[i] = visible_width(header);
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let w = visible_width(cell);
            if w > col_widths[i] {
                col_widths[i] = w;
            }
        }
    }
    let padded_widths: Vec<usize> = col_widths.iter().map(|&w| w + 2).collect();

    // Top border
    print!("┌");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┬");
        }
    }
    println!("┐");

    // Header row
    print!("│");
    for (i, header) in headers.iter().enumerate() {
        let content_width = padded_widths[i];
        let centered = center_to_width(header, content_width);
        print!("{}", centered);
        if i < headers.len() - 1 {
            print!("│");
        }
    }
    println!("│");

    // Separator
    print!("├");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┼");
        }
    }
    println!("┤");

    // Data rows
    for (idx, row) in rows.iter().enumerate() {
        print!("│");
        for (i, cell) in row.iter().enumerate() {
            let content_width = padded_widths[i];
            let centered = center_to_width(cell, content_width);
            print!("{}", centered);
            if i < row.len() - 1 {
                print!("│");
            }
        }
        println!("│");
        if idx < rows.len() - 1 {
            print!("├");
            for (i, &w) in padded_widths.iter().enumerate() {
                print!("{}", "─".repeat(w));
                if i < padded_widths.len() - 1 {
                    print!("┼");
                }
            }
            println!("┤");
        }
    }

    // Bottom border
    print!("└");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┴");
        }
    }
    println!("┘");
}

// --- Print Daily Table (all columns centered) ---
fn print_daily_table(rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }

    let headers = ["Date", "Weather", "Min", "Max", "Feel", "Rain"];
    let mut col_widths = vec![0; headers.len()];

    for (i, header) in headers.iter().enumerate() {
        col_widths[i] = visible_width(header);
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let w = visible_width(cell);
            if w > col_widths[i] {
                col_widths[i] = w;
            }
        }
    }
    let padded_widths: Vec<usize> = col_widths.iter().map(|&w| w + 2).collect();

    // Top border
    print!("┌");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┬");
        }
    }
    println!("┐");

    // Header row
    print!("│");
    for (i, header) in headers.iter().enumerate() {
        let content_width = padded_widths[i];
        let centered = center_to_width(header, content_width);
        print!("{}", centered);
        if i < headers.len() - 1 {
            print!("│");
        }
    }
    println!("│");

    // Separator
    print!("├");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┼");
        }
    }
    println!("┤");

    // Data rows
    for (idx, row) in rows.iter().enumerate() {
        print!("│");
        for (i, cell) in row.iter().enumerate() {
            let content_width = padded_widths[i];
            let centered = center_to_width(cell, content_width);
            print!("{}", centered);
            if i < row.len() - 1 {
                print!("│");
            }
        }
        println!("│");
        if idx < rows.len() - 1 {
            print!("├");
            for (i, &w) in padded_widths.iter().enumerate() {
                print!("{}", "─".repeat(w));
                if i < padded_widths.len() - 1 {
                    print!("┼");
                }
            }
            println!("┤");
        }
    }

    // Bottom border
    print!("└");
    for (i, &w) in padded_widths.iter().enumerate() {
        print!("{}", "─".repeat(w));
        if i < padded_widths.len() - 1 {
            print!("┴");
        }
    }
    println!("┘");
}

// --- Main Program Loop ---
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut current_location: Option<GeocodingResult> = None;
    let mut current_weather: Option<WeatherResponse> = None;

    loop {
        if current_location.is_none() {
            let city_query = loop {
                print!("Enter a location: ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let trimmed = input.trim();
                if !trimmed.is_empty() {
                    break trimmed.to_string();
                }
                println!("Location cannot be empty. Please try again.\n");
            };

            println!("\nLooking up: {}...", city_query.cyan());
            let geocode_url = format!(
                "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
                city_query
            );
            let geocode_response: GeocodingResponse = reqwest::get(&geocode_url).await?.json().await?;
            let location = geocode_response
                .results
                .as_ref()
                .and_then(|r| r.first())
                .ok_or("Could not find location")?
                .clone();
            println!(
                "Found: {}, {} ({:.2}, {:.2})\n",
                location.name.green(),
                location.country.green(),
                location.latitude,
                location.longitude
            );
            current_location = Some(location);
            current_weather = None;
        }

        let loc = current_location.as_ref().unwrap();

        if current_weather.is_none() {
            println!("Fetching weather data...");
            let weather_url = format!(
                "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&hourly=temperature_2m,apparent_temperature,weathercode,precipitation_probability,windspeed_10m&daily=temperature_2m_max,temperature_2m_min,apparent_temperature_max,apparent_temperature_min,weathercode,precipitation_probability_max&timezone=auto&forecast_days=7",
                loc.latitude, loc.longitude
            );
            let weather_response: WeatherResponse = reqwest::get(&weather_url).await?.json().await?;
            current_weather = Some(weather_response);
            println!("Weather data loaded.\n");
        }

        let weather = current_weather.as_ref().unwrap();

        print!("Hourly (h) or Daily (d) forecast? [h/d]: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim().to_lowercase();

        if choice == "d" {
            let daily = &weather.daily;
            println!(
                "\n{}",
                format!("7-Day Forecast for {}, {}", loc.name, loc.country)
                    .bold()
                    .underline()
            );
            println!();

            let mut rows = Vec::new();
            let dates: Vec<NaiveDate> = daily
                .time
                .iter()
                .map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
                .collect();

            for i in 0..daily.time.len() {
                let date_str = dates[i].format("%a %d %b").to_string();
                let weather_text = get_weather_text(daily.weathercode[i]);
                let weather_colour = get_weather_colour(daily.weathercode[i]);
                let coloured_weather = weather_text.color(weather_colour).to_string();
                let min = format_temp(daily.temperature_2m_min[i], Color::Green);
                let max = format_temp(daily.temperature_2m_max[i], Color::Red);
                let feels = format_feels(daily.apparent_temperature_max[i]);
                let rain = format_rain(daily.precipitation_probability_max[i].unwrap_or(0));
                rows.push(vec![date_str, coloured_weather, min, max, feels, rain]);
            }

            print_daily_table(&rows);
        } else {
            let hourly = &weather.hourly;
            let groups = group_hourly_by_day(
                &hourly.time,
                &hourly.temperature_2m,
                &hourly.apparent_temperature,
                &hourly.weathercode,
                &hourly.precipitation_probability,
                &hourly.windspeed_10m,
            );

            let mut sorted_dates: Vec<String> = groups.keys().cloned().collect();
            sorted_dates.sort();
            sorted_dates.truncate(7);

            println!("\nAvailable days for hourly forecast:");
            for (idx, date) in sorted_dates.iter().enumerate() {
                let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
                let day_name = parsed.format("%A, %d %B %Y").to_string();
                println!("  [{}] {}", idx + 1, day_name);
            }

            let selected_date = loop {
                print!("\nEnter day number (1-{}): ", sorted_dates.len());
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let input = input.trim();
                if let Ok(num) = input.parse::<usize>() {
                    if num >= 1 && num <= sorted_dates.len() {
                        break sorted_dates[num - 1].clone();
                    }
                }
                println!("Invalid input. Please enter a number between 1 and {}.", sorted_dates.len());
            };

            let (hours, temps, feels, codes, precip, wind) = &groups[&selected_date];
            let min_temp = temps.iter().cloned().fold(f64::INFINITY, f64::min);
            let max_temp = temps.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

            let parsed_date = NaiveDate::parse_from_str(&selected_date, "%Y-%m-%d").unwrap();
            println!(
                "\n{}",
                format!(
                    "Hourly Forecast for {}, {} – {}",
                    loc.name,
                    loc.country,
                    parsed_date.format("%A, %d %B %Y")
                )
                    .bold()
                    .underline()
            );
            println!();

            let mut rows = Vec::new();
            for i in 0..hours.len() {
                let hour = hours[i].clone();
                let weather_text = get_weather_text(codes[i]);
                let weather_colour = get_weather_colour(codes[i]);
                let coloured_weather = weather_text.color(weather_colour).to_string();
                let temp = temps[i];
                let feel = feels[i];
                let temp_plain = format!("{:.1}°C", temp);
                let temp_padded = pad_number(&temp_plain);
                // No color on temperature number (default terminal color)
                let temp_no_color = temp_padded;  // just the padded string
                let bar = draw_temp_bar_compact(temp, min_temp, max_temp);
                let temp_display = format!("{} {}", temp_no_color, bar);
                let feel_str = format_feels(feel);
                let rain_str = format_rain(precip[i]);
                let wind_str = format_wind(wind[i]);
                rows.push(vec![hour, coloured_weather, temp_display, feel_str, rain_str, wind_str]);
            }

            print_hourly_table(&rows);
        }

        // Main menu loop
        loop {
            println!();
            println!("{}", "─".repeat(40));
            println!("What would you like to do?");
            println!("  1. New location");
            println!("  2. Same location, different forecast");
            println!("  0. Exit");
            print!("Choice: ");
            io::stdout().flush()?;
            let mut menu_choice = String::new();
            io::stdin().read_line(&mut menu_choice)?;
            let menu_choice = menu_choice.trim();

            match menu_choice {
                "1" => {
                    current_location = None;
                    current_weather = None;
                    println!("\n");
                    break;
                }
                "2" => {
                    println!("\n");
                    break;
                }
                "0" => {
                    println!("Goodbye!");
                    return Ok(());
                }
                _ => {
                    println!("Invalid choice. Please enter 1, 2, or 0.");
                }
            }
        }
    }
}