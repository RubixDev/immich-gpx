use std::{fs::File, io::BufReader, path::PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use itertools::Itertools;
use reqwest::{
    Client,
    header::{HeaderName, HeaderValue},
};
use serde_json::json;

#[derive(clap::Parser)]
struct Cli {
    /// Paths to gpx input files.
    gpx_files: Vec<PathBuf>,

    /// URL to Immich server. E.g. `https://immich.example.com`
    #[clap(long)]
    server: String,

    /// Don't actually send updates to Immich.
    #[clap(short = 'n', long)]
    dry_run: bool,

    /// Only apply to assets owned by user with this ID.
    #[clap(long)]
    owner: Option<String>,

    /// Only apply to assets taken with a camera of this brand.
    #[clap(long)]
    camera_brand: Option<String>,

    /// Only apply to assets taken with this camera model.
    #[clap(long)]
    camera_model: Option<String>,

    /// Page number when searching assets.
    ///
    /// Pages are usually 250 items each, so by default only the latest 250
    /// pictures without location data will be processed.
    #[clap(short, long, default_value = "1")]
    page: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let api_key = dotenv::var("IMMICH_API_KEY").context("missing Immich API Key")?;

    let mut location_data = args
        .gpx_files
        .iter()
        .map(|path| {
            Result::Ok(
                gpx::read(BufReader::new(
                    File::open(path).context("could not open specified file for reading")?,
                ))
                .context("could not read gpx data")?
                .tracks
                .into_iter()
                .flat_map(|track| track.segments)
                .map(|segment| {
                    segment
                        .points
                        .into_iter()
                        .filter_map(|p| Some(convert_time(p.time?).map(|t| (t, p.point().x_y()))))
                        .collect::<Result<Vec<_>>>()
                }),
            )
        })
        .flatten_ok()
        .flatten_ok()
        .collect::<Result<Vec<_>>>()?;

    for segment in &mut location_data {
        segment.sort_unstable_by_key(|p| p.0);
    }

    let client = Client::builder()
        .default_headers(
            [(
                HeaderName::from_static("x-api-key"),
                HeaderValue::from_str(&api_key).context("API key must be ASCII")?,
            )]
            .into_iter()
            .collect(),
        )
        .build()?;
    let base_url = format!("{}/api", args.server);
    let images = client
        .post(format!("{base_url}/search/metadata"))
        .json(&json!({
            "page": args.page,
            "withExif": true,
            "country": null,
            "make": args.camera_brand,
            "model": args.camera_model,
        }))
        .send()
        .await
        .context("failed to get assets from immich")?
        .json::<SearchResult>()
        .await?;

    for image in images.assets.items.into_iter().filter(|img| {
        args.owner.as_ref().is_none_or(|id| id == &img.owner_id)
            && img.exif_info.latitude.is_none()
            && img.exif_info.longitude.is_none()
    }) {
        // find track including this time, if any
        let Some(track) = location_data
            .iter()
            .filter(|track| !track.is_empty())
            .find(|track| {
                image.exif_info.date_time_original >= track.first().unwrap().0
                    && image.exif_info.date_time_original <= track.last().unwrap().0
            })
        else {
            continue;
        };

        // find closest positions
        let [a, b] = track
            .iter()
            // in case the last point is exactly at when the image was taken
            .chain(std::iter::once(track.last().unwrap()))
            .skip_while(|p| p.0 < image.exif_info.date_time_original)
            .take(2)
            .collect_array()
            .expect("track should contain at least two points not before image capture");

        // lerp position based on capture time
        let points_dt = (b.0 - a.0).num_seconds().max(1) as f64;
        let capture_dt = (image.exif_info.date_time_original - a.0)
            .num_seconds()
            .max(1) as f64;
        let longitude = a.1.0 + ((b.1.0 - a.1.0) * capture_dt / points_dt);
        let latitude = a.1.1 + ((b.1.1 - a.1.1) * capture_dt / points_dt);

        // set location info
        println!(
            "setting location {latitude}, {longitude} for image {}/photos/{}",
            args.server, image.id,
        );
        if !args.dry_run {
            client
                .put(format!("{base_url}/assets/{}", image.id))
                .json(&json!({
                    "latitude": latitude,
                    "longitude": longitude,
                }))
                .send()
                .await
                .context("failed to update asset")?;
        }
    }

    Ok(())
}

fn convert_time(time: gpx::Time) -> Result<DateTime<Utc>> {
    Ok(
        DateTime::parse_from_rfc3339(&time.format().context("failed to format time as string")?)
            .context("failed to parse string as datetime")?
            .to_utc(),
    )
}

#[derive(Debug, Clone, serde::Deserialize)]
struct SearchResult {
    assets: SearchAssetResponseDto,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct SearchAssetResponseDto {
    items: Vec<AssetResponseDto>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetResponseDto {
    id: String,
    exif_info: ExifResponseDto,
    owner_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExifResponseDto {
    date_time_original: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}
