use anyhow::{bail, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{base::cmc_id, price_history::PricePoint};

// struct to deserialize the following json data:
// {"data":{"id":1,"name":"Bitcoin","symbol":"BTC","timeEnd":"1259279999","quotes":[{"timeOpen":"2010-07-13T00:00:00.000Z","timeClose":"2010-07-13T23:59:59.999Z","timeHigh":"2010-07-13T02:30:00.000Z","timeLow":"2010-07-13T18:06:00.000Z","quote":{"open":0.0487103725,"high":0.0609408726,"low":0.0483279245,"close":0.0534224523,"volume":59.4135378071,"marketCap":153229.6094699791,"timestamp":"2010-07-13T23:59:59.999Z"}},{"timeOpen":"2010-07-14T00:00:00.000Z","timeClose":"2010-07-14T23:59:59.999Z","timeHigh":"2010-07-14T00:34:00.000Z","timeLow":"2010-07-14T19:24:00.000Z","quote":{"open":0.0534167944,"high":0.0565679556,"low":0.0446813119,"close":0.0518047635,"volume":240.2216130600,"marketCap":174751.3956688508,"timestamp":"2010-07-14T23:59:59.999Z"}},{"timeOpen":"2010-07-15T00:00:00.000Z","timeClose":"2010-07-15T23:59:59.999Z","timeHigh":"2010-07-15T11:39:00.000Z","timeLow":"2010-07-15T00:41:00.000Z","quote":{"open":0.0518051769,"high":0.0624153413,"low":0.0495701257,"close":0.0528756482,"volume":409.4623962000,"marketCap":180007.4397864608,"timestamp":"2010-07-15T23:59:59.999Z"}},{"timeOpen":"2010-07-16T00:00:00.000Z","timeClose":"2010-07-16T23:59:59.999Z","timeHigh":"2010-07-16T02:11:00.000Z","timeLow":"2010-07-16T00:24:00.000Z"}}]},"status":{"timestamp":"2023-08-30T09:28:02.491Z","error_code":"0","error_message":"SUCCESS","elapsed":"74","credit_count":0}}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CmcHistoricalDataResponse {
    data: CmcHistoricalData,
    // status: CmcResponseStatus,
}

// #[derive(Debug, Deserialize)]
// struct CmcResponseStatus {
//     timestamp: DateTime<FixedOffset>,
//     error_code: String,
//     error_message: String,
//     elapsed: String,
//     credit_count: i64,
// }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CmcHistoricalData {
    // id: i32,
    // name: String,
    // symbol: String,
    // time_end: String,
    quotes: Vec<CmcQuote>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CmcQuote {
    time_open: DateTime<FixedOffset>,
    // time_close: DateTime<FixedOffset>,
    // time_high: DateTime<FixedOffset>,
    // time_low: DateTime<FixedOffset>,
    quote: Quote,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Quote {
    open: Decimal,
    // high: Decimal,
    // low: Decimal,
    // close: Decimal,
    // volume: Decimal,
    // market_cap: Decimal,
    // timestamp: DateTime<FixedOffset>,
}

#[allow(dead_code)]
#[derive(Copy, Clone)]
pub(crate) enum CmcInterval {
    Hourly,
    Daily,
    Weekly,
}

impl CmcInterval {
    fn as_str(&self) -> &str {
        match self {
            CmcInterval::Hourly => "1h",
            CmcInterval::Daily => "1d",
            CmcInterval::Weekly => "7d",
        }
    }

    pub(crate) fn duration(&self) -> Duration {
        match self {
            CmcInterval::Hourly => Duration::hours(1),
            CmcInterval::Daily => Duration::days(1),
            CmcInterval::Weekly => Duration::weeks(1),
        }
    }
}

pub(crate) async fn download_price_points(
    time_start: NaiveDateTime,
    time_end: NaiveDateTime,
    currency: &str,
    interval: CmcInterval,
) -> Result<Vec<PricePoint>> {
    let id = cmc_id(currency);
    if id == -1 {
        bail!("Unsupported currency (cmc id not known): {}", currency);
    }

    let convert_id = cmc_id("EUR");
    let time_start: i64 = time_start.and_utc().timestamp();
    let mut time_end: i64 = time_end.and_utc().timestamp();
    // make sure time_end isn't in the future
    if time_end > Utc::now().timestamp() {
        time_end = Utc::now().timestamp();
    }
    let url = format!(
        "https://api.coinmarketcap.com/data-api/v3.3/cryptocurrency/historical?id={}&convertId={}&timeStart={}&timeEnd={}&interval={}",
        id, convert_id, time_start, time_end, interval.as_str()
    );
    println!("Downloading {}", url);

    let response: CmcHistoricalDataResponse = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/118.0")
        .build()?
        .get(url.clone())
        .send()
        .await?
        .json()
        .await?;

    let prices: Vec<PricePoint> = response
        .data
        .quotes
        .into_iter()
        .map(|quote| PricePoint {
            timestamp: quote.time_open.naive_utc(),
            price: quote.quote.open,
        })
        .collect();

    println!("Downloaded {} price points", prices.len());
    Ok(prices)
}

#[allow(dead_code)]
pub(crate) async fn download_price_history(currency: &str) -> Result<()> {
    let id = cmc_id(currency);
    if id == -1 {
        println!("Unsupported currency (cmc id not known): {}", currency);
        return Ok(());
    }

    let mut prices: Vec<PricePoint> = Vec::new();

    for year in 2010..2024 {
        let time_start = DateTime::parse_from_rfc3339(&format!("{}-01-01T00:00:00+00:00", year))
            .unwrap()
            .naive_utc();
        let time_end = DateTime::parse_from_rfc3339(&format!("{}-12-31T23:59:59+00:00", year))
            .unwrap()
            .naive_utc();

        let mut year_prices =
            download_price_points(time_start, time_end, currency, CmcInterval::Daily).await?;
        let price_point_count = year_prices.len();
        prices.append(&mut year_prices);

        println!("Loaded {} price points for {}", price_point_count, year);
    }

    let path = format!("src/data/{}-price-history-eur.csv", currency.to_lowercase());
    println!("Saving {} price points to {}", prices.len(), path);
    crate::price_history::save_price_history_data(&prices, path.as_ref())?;

    Ok(())
}
