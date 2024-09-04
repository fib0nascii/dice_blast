use io::Error;
use std::collections::{HashMap, HashSet};
use thirtyfour::error::WebDriverError;
use thirtyfour::error::WebDriverErrorInfo;
use thirtyfour::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{from_reader, json, Value};
use serde_urlencoded;
use std::path::Path;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::io::Write;
use std::io::Result;
use std::io;
use std::fmt;
use std::fmt::Display;
use regex::Regex;
use tokio::time::{Duration};
use base64::{encode_config};
use base64::URL_SAFE;
use thirtyfour::support::sleep;

#[derive(Serialize, Deserialize)]
struct Cookie {
    name: String,
    value: String,
    domain: Option<String>,
    path: Option<String>,
    expiry: Option<u64>,
    secure: bool,
    http_only: Option<bool>, // Make this field optional
}

#[derive(Serialize, Deserialize)]
struct SearchQuery {
    q: String,
    location: String,
    #[serde(rename = "countryCode")]
    country_code: String,
    #[serde(rename = "filters.employmentType")]
    filters_employment_type: String,
    #[serde(rename = "filters.employerType")]
    filters_employer_type: String,
    #[serde(rename = "filters.easyApply")]
    filters_easy_apply: bool, 
    language: String
}


#[derive(Debug)]
enum ConfigError {
    FileError(Error),
    ParseError(serde_json::Error),
    UrlEncoded(serde_urlencoded::ser::Error),
}

#[derive(Debug)]
struct Job {
    page_number: usize,
    job_title: String,
    url: String,
}
impl Display for SearchQuery {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "q: {}, location: {}, country_code: {}, filters_employer_type: {}, filters_easy_apply: {}, language: {}",
            self.q,
            self.location,
            self.country_code,
            self.filters_employer_type,
            self.filters_easy_apply,
            self.language
        )
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ConfigError::FileError(ref err) => write!(f, "File error: {}", err),
            ConfigError::ParseError(ref err) => write!(f, "Parse error: {}", err),
            ConfigError::UrlEncoded(ref err) => write!(f, "URL encoding error: {}", err),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            ConfigError::FileError(ref err) => Some(err),
            ConfigError::ParseError(ref err) => Some(err),
            ConfigError::UrlEncoded(ref err) => Some(err),
        }
    }
}

impl From<Error> for ConfigError {
    fn from(err: Error) -> ConfigError {
        ConfigError::FileError(err)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(err: serde_json::Error) -> ConfigError {
        ConfigError::ParseError(err)
    }
}

impl From<serde_urlencoded::ser::Error> for ConfigError {
    fn from(err: serde_urlencoded::ser::Error) -> ConfigError {
        ConfigError::UrlEncoded(err)
    }
}


fn build_url_from_config() -> Result<String> {
    println!("Building search url from config file...");
    let file = File::open("./config.json")?;
    let reader = BufReader::new(file);
    let search_query: SearchQuery = from_reader(reader)?;
    let encoded_query = serde_urlencoded::to_string(&search_query).map_err(ConfigError::UrlEncoded);
    let url = format!("https://dice.com/jobs?{:?}", encoded_query);

    println!("Formatted URL: {}", url);

    Ok(url)
}


async fn load_cookies(driver: &WebDriver) -> WebDriverResult<()> {
    let file = File::open("cookies.json")?;
    let reader = BufReader::new(file);
    let cookies: Vec<Cookie> = from_reader(reader)?;

    for cookie in cookies {
        let web_cookie = thirtyfour::cookie::Cookie {
            name: cookie.name,
            value: cookie.value,
            domain: cookie.domain,
            path: cookie.path,
            expiry: cookie.expiry.map(|e| e as i64),
            secure: Some(cookie.secure),
            same_site: None,
        };
        driver.add_cookie(web_cookie).await?;
    }

    // println!("Press Enter to exit...");
    // let _ = io::stdout().flush();
    // let _ = io::stdin().read_line(&mut String::new());

    Ok(())
}

async fn save_cookies(driver: &WebDriver) -> WebDriverResult<()> {
    let cookies = driver.get_all_cookies().await?;
    let file = File::create("cookies.json")?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, &cookies)?;
    Ok(())
}

async fn login(driver: &WebDriver) -> WebDriverResult<()> {
    // Navigate to Dice Login Page
    driver.get("https://dice.com/dashboard/login").await?;

    // Login: Email Field ID: react-aria7000922841-:r0:

    // Wait for user input to keep the browser open
    // After logging in Press Enter to exit so the next function can run to fetch the cookie
    println!("Press Enter to exit...");
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut String::new());

    Ok(())
}

fn cookie_exists() -> Result<bool> {
    let cookie_file = Path::new("./cookies.json");
    match File::open(cookie_file) {
        Ok(_) => {
            println!("There is an existing cookie file. Continuing with program execution.");
            Ok(true)
        }
        Err(_) => {
            println!("There is no existing cookie file. Continuing to login.");
            Ok(false)
        }
    }
}

//Job Detail Pages look like https://www.dice.com/job-detail/f0767d15-68a2-4c23-95c6-5685dedf2d2d
// Need to grab the IDs for each job and append them on a future page

async fn wait_for_element(driver: &WebDriver, selector: By, timeout: Duration) -> WebDriverResult<()> {
    let start = tokio::time::Instant::now();
    loop {
        if tokio::time::Instant::now() - start > timeout {
            return Err(WebDriverError::Timeout("Timeout waiting for element".into()));
        }
        if driver.find(selector.clone()).await.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(500)).await;
    }
}

async fn get_job_detail_ids(driver: &WebDriver, page_number: usize) -> WebDriverResult<Vec<Job>> {
    // Wait for the page to load
    wait_for_element(driver, By::Css("div"), Duration::from_secs(30)).await?;
    sleep(Duration::from_secs(5)).await; // Additional delay to ensure the page is fully loaded

    println!("Finding elements...");
    let div_elements = driver.find_all(By::Css("div")).await?;
    println!("Found {} <div> elements", div_elements.len());

    let mut jobs = Vec::new();
    let mut seen_ids = HashSet::new();
    let hash_pattern = Regex::new(r"^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$").unwrap();

    for div in div_elements {
        let a_elements = div.find_all(By::Css("a")).await?;
        for a in a_elements {
            if let Ok(id) = a.attr("id").await {
                if let Some(id_value) = id {
                    if hash_pattern.is_match(&id_value) && !seen_ids.contains(&id_value) {
                        if let Ok(title) = a.text().await {
                            println!("Job Title: {}, Job ID: {}", title, id_value);
                            let job = Job {
                                page_number,
                                job_title: title,
                                url: format!("https://dice.com/job-detail/{}", id_value),
                            };
                            jobs.push(job);
                            seen_ids.insert(id_value);
                        }
                    }
                }
            }
        }
    }

    Ok(jobs)
}

fn generate_encoded_url(job_id: &str, job_title: &str, search_params: &str) -> String {
    let data = json!({
        "djvVersion": "new",
        "jobId": job_id,
        "jobUrl": format!("https://www.dice.com/job-detail/{}?{}", job_id, search_params),
        "jobTitle": job_title,
        "searchLink": format!("?searchlink=search%2F%3F{}", search_params),
        "searchParams": format!("?{}", search_params)
    });
    let json_data = serde_json::to_string(&data).unwrap();
    let encoded_data = encode_config(json_data, URL_SAFE);
    format!("https://www.dice.com/apply?{}", encoded_data)
}

async fn open_job_urls(driver: &WebDriver, jobs: Vec<Job>, search_params: &str) -> WebDriverResult<()> {
    for job in jobs {
        println!("Opening job URL: {}", job.url);
        let encoded_url = generate_encoded_url(&job.url, &job.job_title, search_params);
        println!("Navigating to encoded URL: {}", encoded_url);

        // Load cookies from the file
        load_cookies(driver).await?;

        driver.get(&encoded_url).await?;
        sleep(Duration::from_secs(10)).await; // Wait for 10 seconds to ensure the page is fully loaded

        // Click the "Easy Apply" button using JavaScript
        let script_easy_apply = r#"
            var button = document.querySelector('button.btn.btn-primary');
            if (button && button.innerText === 'Easy apply') {
                button.click();
            }
        "#;
        driver.execute(script_easy_apply, vec![]).await?;
        sleep(Duration::from_secs(10)).await; // Wait for 10 seconds to ensure the application page is fully loaded

        // Wait for the "Next" button to be present and clickable
        let script_wait_next_button = r#"
            return new Promise((resolve) => {
                const observer = new MutationObserver((mutations, obs) => {
                    const nextButton = document.querySelector('button.seds-button-primary.btn-next');
                    if (nextButton && nextButton.innerText === 'Next') {
                        obs.disconnect();
                        resolve(true);
                    }
                });
                observer.observe(document, { childList: true, subtree: true });
            });
        "#;
        driver.execute(script_wait_next_button, vec![]).await?;
        sleep(Duration::from_secs(2)).await; // Wait for 2 seconds to ensure the button is fully interactable

        // Click the "Next" button using JavaScript
        let script_next_button = r#"
            var nextButton = document.querySelector('button.seds-button-primary.btn-next');
            if (nextButton && nextButton.innerText === 'Next') {
                nextButton.click();
            }
        "#;
        driver.execute(script_next_button, vec![]).await?;
        sleep(Duration::from_secs(10)).await; // Wait for 10 seconds to ensure the next page is fully loaded

        // Wait for the "Submit" button to be present and clickable
        let script_wait_submit_button = r#"
            return new Promise((resolve) => {
                const observer = new MutationObserver((mutations, obs) => {
                    const submitButton = document.querySelector('button.seds-button-primary.btn-next');
                    if (submitButton && submitButton.innerText === 'Submit') {
                        obs.disconnect();
                        resolve(true);
                    }
                });
                observer.observe(document, { childList: true, subtree: true });
            });
        "#;
        driver.execute(script_wait_submit_button, vec![]).await?;
        sleep(Duration::from_secs(2)).await; // Wait for 2 seconds to ensure the button is fully interactable

        // Click the "Submit" button using JavaScript
        let script_submit_button = r#"
            var submitButton = document.querySelector('button.seds-button-primary.btn-next');
            if (submitButton && submitButton.innerText === 'Submit') {
                submitButton.click();
            }
        "#;
        driver.execute(script_submit_button, vec![]).await?;
        sleep(Duration::from_secs(10)).await; // Wait for 10 seconds to ensure the application is submitted

        // Wait for 2 seconds before opening the next URL
        sleep(Duration::from_secs(2)).await;
    }
    Ok(())
}




#[tokio::main]
async fn main() -> WebDriverResult<()> {
    let caps = DesiredCapabilities::chrome();
    let driver = WebDriver::new("http://localhost:9415", caps).await?;
    let url = build_url_from_config()?; // Unwrap the URL here
    let login_result = login(&driver).await;

    match cookie_exists() {
        Ok(true) => {
            // Continue program execution
            load_cookies(&driver).await?;
            driver.get(&url).await?;
            let jobs = get_job_detail_ids(&driver, 1).await?;
            open_job_urls(&driver, jobs, "").await?;

            println!("Press Enter to exit...");
            let _ = io::stdout().flush();
            let _ = io::stdin().read_line(&mut String::new());

            Ok(())
        }
        Ok(false) => {
            match login_result {
                Ok(()) => {
                    save_cookies(&driver).await?;
                    driver.get(&url).await?;
                    let jobs = get_job_detail_ids(&driver, 1).await?;
                    open_job_urls(&driver, jobs, "").await?;

                    println!("Press Enter to exit...");
                    let _ = io::stdout().flush();
                    let _ = io::stdin().read_line(&mut String::new());

                    Ok(())
                }
                Err(e) => {
                    println!("Something went wrong! Please try again... Error: {:?}", e);
                    Err(WebDriverError::UnknownError(WebDriverErrorInfo::new("Login failed".to_string())))
                }
            }
        }
        Err(e) => {
            println!("Error checking cookie file: {:?}", e);
            Err(WebDriverError::UnknownError(WebDriverErrorInfo::new("Error checking cookie file".to_string())))
        }
    }
}
