use std::{error::Error, path::Path};

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde::{Deserialize, Serialize};

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
    open: f64,
    // high: f64,
    // low: f64,
    // close: f64,
    // volume: f64,
    // market_cap: f64,
    // timestamp: DateTime<FixedOffset>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PricePoint {
    timestamp: NaiveDateTime,
    price: f64,
}

// command to download BTC price history for 2023:
// curl "https://api.coinmarketcap.com/data-api/v3.1/cryptocurrency/historical?id=1&convertId=2790&timeStart=1672527600&timeEnd=1704063600&interval=1d" -o btc-price-history-2023.json
pub(crate) fn load_btc_price_history_data_json() -> Result<Vec<PricePoint>, Box<dyn Error>> {
    let mut prices: Vec<PricePoint> = Vec::new();
    let base = "btc-price-history-";

    for year in 2010..2024 {
        let file = base.to_owned() + &year.to_string() + ".json";
        let response: CmcHistoricalDataResponse = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        let price_point_count = response.data.quotes.len();

        for quote in response.data.quotes {
            prices.push(PricePoint {
                timestamp: quote.time_open.naive_utc(),
                price: quote.quote.open,
            });
        }

        println!("Loaded {} price points from {}", price_point_count, file);
    }

    Ok(prices)
}

pub(crate) fn save_btc_price_history_data(prices: &Vec<PricePoint>, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::Writer::from_path(path)?;
    for price in prices {
        wtr.serialize(price)?;
    }

    Ok(())
}

pub(crate) fn load_btc_price_history_data() -> Result<Vec<PricePoint>, Box<dyn Error>> {
    // The following file was saved using the above function with data loaded
    // from the CoinMarketCap API.
    let btc_price_history_eur = include_bytes!("data/btc-price-history-eur.csv");

    let mut rdr = csv::Reader::from_reader(btc_price_history_eur.as_slice());
    let mut prices: Vec<PricePoint> = Vec::new();
    for result in rdr.deserialize() {
        let record: PricePoint = result?;
        prices.push(record);
    }
    Ok(prices)
}

pub(crate) fn estimate_btc_price(time: NaiveDateTime, prices: &Vec<PricePoint>) -> Option<f64> {
    let index = prices.partition_point(|p| p.timestamp < time);
    let next_price_point = prices.get(index).or_else(|| prices.last());
    let prev_price_point = if index > 0 { prices.get(index - 1) } else { None };

    if let (Some(next_price), Some(prev_price)) = (next_price_point, prev_price_point) {
        // calculate the most probable price, by linear iterpolation based on the previous and next price
        let price_difference = next_price.price - prev_price.price;

        let total_duration = (next_price.timestamp - prev_price.timestamp).num_seconds() as f64;
        let time_since_prev = (time - prev_price.timestamp).num_seconds() as f64;
        let time_ratio = time_since_prev / total_duration;

        Some(prev_price.price + time_ratio * price_difference)
    } else {
        None
    }
}
