use anyhow::{Result, bail};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::base::{cmc_id, PricePoint, save_price_history_data};

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
pub(crate) async fn download_price_points(time_end: NaiveDateTime, currency: &str) -> Result<Vec<PricePoint>> {
    let id = cmc_id(currency);
    if id == -1 {
        bail!("Unsupported currency (cmc id not known): {}", currency);
    }

    let convert_id = cmc_id("EUR");
    let time_end: i64 = time_end.timestamp();
    let url = format!("https://api.coinmarketcap.com/data-api/v3.1/cryptocurrency/historical?id={}&convertId={}&timeEnd={}&interval=1h", id, convert_id, time_end);
    println!("Downloading {}", url);

    let response: CmcHistoricalDataResponse = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/118.0")
        .build()?.get(url.clone()).send().await?.json().await?;

    let prices: Vec<PricePoint> = response.data.quotes
        .into_iter()
        .map(|quote| PricePoint {
            timestamp: quote.time_open.naive_utc(),
            price: quote.quote.open,
        })
        .collect();

    // Fill the prices with hourly dummy data spanning two weeks before time_end
    // let mut prices: Vec<PricePoint> = Vec::new();
    // let time_start = time_end - 2 * 7 * 24 * 3600;
    // for hour in 0..(2 * 7 * 24) {
    //     let time = time_start + hour * 3600;
    //     prices.push(PricePoint {
    //         timestamp: NaiveDateTime::from_timestamp_opt(time, 0).unwrap(),
    //         price: Decimal::ZERO,
    //     });
    // }

    println!("Loaded {} price points from {}", prices.len(), url);
    Ok(prices)
}

#[allow(dead_code)]
pub(crate) async fn download_price_history(currency: &str) -> Result<()> {
    let id = cmc_id(currency);
    if id == -1 {
        println!("Unsupported currency (cmc id not known): {}", currency);
        return Ok(());
    }

    let convert_id = cmc_id("EUR");

    let mut prices: Vec<PricePoint> = Vec::new();

    for year in 2010..2024 {
        let time_start = DateTime::parse_from_rfc3339(&format!("{}-01-01T00:00:00+00:00", year)).unwrap().timestamp();
        let time_end = DateTime::parse_from_rfc3339(&format!("{}-12-31T23:59:59+00:00", year)).unwrap().timestamp();
        let url = format!("https://api.coinmarketcap.com/data-api/v3.1/cryptocurrency/historical?id={}&convertId={}&timeStart={}&timeEnd={}&interval=1d", id, convert_id, time_start, time_end);
        println!("Downloading {}", url);

        let response: CmcHistoricalDataResponse = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/118.0")
            .build()?.get(url.clone()).send().await?.json().await?;

        let price_point_count = response.data.quotes.len();

        for quote in response.data.quotes {
            prices.push(PricePoint {
                timestamp: quote.time_open.naive_utc(),
                price: quote.quote.open,
            });
        }

        println!("Loaded {} price points from {}", price_point_count, url);
    }

    let path = format!("src/data/{}-price-history-eur.csv", currency.to_lowercase());
    println!("Saving {} price points to {}", prices.len(), path);
    save_price_history_data(&prices, path.as_ref())?;

    Ok(())
}
