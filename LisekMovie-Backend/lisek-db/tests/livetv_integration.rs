//! Database-backed tests for Faza 20 Live TV: channels keep their ids
//! across refreshes, the guide window, sources due for a refresh.
//! Opt-in: see `common/mod.rs`.

use chrono::{Duration, Utc};
use lisek_db::livetv::{self, NewChannel, NewProgram};

mod common;
use common::TestDb;

fn channel(key: &str, name: &str) -> NewChannel {
    NewChannel {
        key: key.into(),
        tvg_id: Some(key.into()),
        name: name.into(),
        number: None,
        group_title: Some("Ogólne".into()),
        logo_url: Some("https://logo/x.png".into()),
        stream_url: format!("http://iptv/{key}.ts"),
    }
}

#[tokio::test]
async fn channels_and_guide_are_replaced_on_refresh() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = lisek_db::create_library(pool, "Telewizja", livetv::LIVE_TV, &[], false).await.unwrap().id;
    assert!(lisek_db::create_library(pool, "Zły typ", "radio", &[], false).await.is_err());
    livetv::set_source(pool, lib, "https://iptv/list.m3u", Some("https://iptv/guide.xml")).await.unwrap();
    assert_eq!(livetv::sources_due(pool, 12).await.unwrap(), vec![lib], "never refreshed");

    let now = Utc::now();
    let programs = vec![
        NewProgram { channel: 0, starts_at: now - Duration::minutes(10), ends_at: now + Duration::minutes(20), title: "Teraz".into(), subtitle: None, description: Some("Opis".into()), category: None, icon_url: None },
        NewProgram { channel: 0, starts_at: now + Duration::minutes(20), ends_at: now + Duration::minutes(50), title: "Potem".into(), subtitle: None, description: None, category: None, icon_url: None },
        NewProgram { channel: 1, starts_at: now + Duration::hours(30), ends_at: now + Duration::hours(31), title: "Jutro".into(), subtitle: None, description: None, category: None, icon_url: None },
        NewProgram { channel: 9, starts_at: now, ends_at: now + Duration::hours(1), title: "Bez kanału".into(), subtitle: None, description: None, category: None, icon_url: None },
    ];
    let stored = livetv::replace_catalog(pool, lib, &[channel("tvp1", "TVP 1"), channel("tvn", "TVN")], &programs).await.unwrap();
    assert_eq!(stored, (2, 3));
    let source = livetv::get_source(pool, lib).await.unwrap().unwrap();
    assert_eq!((source.channel_count, source.program_count, source.last_error), (2, 3, None));
    assert!(livetv::sources_due(pool, 12).await.unwrap().is_empty());

    let channels = livetv::channels(pool, None).await.unwrap();
    assert_eq!(channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["TVP 1", "TVN"]);
    let tvp = channels[0].id;
    let window = livetv::programs(pool, None, now, now + Duration::hours(6)).await.unwrap();
    assert_eq!(window.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Teraz", "Potem"]);
    assert!(livetv::channels(pool, Some(&[])).await.unwrap().is_empty(), "restricted to no library");

    // Refresh: TVP 1 renamed and kept, TVN gone, Polsat new.
    livetv::replace_catalog(pool, lib, &[channel("polsat", "Polsat"), channel("tvp1", "TVP 1 HD")], &[]).await.unwrap();
    let channels = livetv::channels(pool, None).await.unwrap();
    assert_eq!(channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Polsat", "TVP 1 HD"]);
    assert_eq!(channels[1].id, tvp, "same channel, same id");
    assert!(livetv::programs(pool, None, now - Duration::days(1), now + Duration::days(2)).await.unwrap().is_empty());
    let source = livetv::channel_source(pool, tvp).await.unwrap().unwrap();
    assert_eq!(source.stream_url, "http://iptv/tvp1.ts");

    livetv::record_error(pool, lib, "guide: download failed").await.unwrap();
    let source = livetv::get_source(pool, lib).await.unwrap().unwrap();
    assert_eq!(source.last_error.as_deref(), Some("guide: download failed"));
    assert_eq!(source.channel_count, 2, "the old catalog stays");

    lisek_db::delete_library(pool, lib).await.unwrap();
    assert!(livetv::list_sources(pool).await.unwrap().is_empty());
    db.drop().await;
}
