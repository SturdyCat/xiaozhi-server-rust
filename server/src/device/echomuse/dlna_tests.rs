use std::time::Duration;

use super::{is_audio, parse_didl, parse_duration, resolve_url};

#[test]
fn didl_parses_items_containers_and_res_attrs() {
    // 注意：真实 DIDL 的 `<res>` 文本是直链，duration 在属性上；这里用规整样例
    // 覆盖 item/container、class、res 属性与实体还原。
    let xml = r#"<DIDL-Lite xmlns="urn:schemas-upnp-org:schemas-UPnP-NS:0">
<container id="0/1" parentID="0" restricted="1"><dc:title>Music</dc:title><upnp:class>object.container.storageFolder</upnp:class></container>
<item id="0/1/2" parentID="0/1" restricted="0"><dc:title>Song A&amp;B</dc:title><upnp:class>object.item.audioItem.musicTrack</upnp:class><upnp:artist>Cool Artist</upnp:artist><upnp:album>Best Of</upnp:album><res duration="0:03:21.000" protocolInfo="http-get:*:audio/mpeg:*">http://192.168.1.8:8200/MediaItems/12.mp3</res></item>
</DIDL-Lite>"#;
    let entries = parse_didl(xml);
    assert_eq!(entries.len(), 2);
    assert!(entries[0].is_container);
    assert_eq!(entries[0].id, "0/1");
    assert!(!entries[1].is_container);
    assert_eq!(entries[1].title, "Song A&B"); // &amp; 被还原
    assert_eq!(entries[1].artist, "Cool Artist");
    assert_eq!(entries[1].album, "Best Of");
    assert_eq!(entries[1].duration, "0:03:21.000");
    assert_eq!(
        entries[1].res_url,
        "http://192.168.1.8:8200/MediaItems/12.mp3"
    );
    assert_eq!(entries[1].res_mime, "audio/mpeg");
    assert!(is_audio(&entries[1].class));
    // 容器不该被当音频条目放行（storageFolder 不含 audio/music/song）
    assert!(!is_audio(&entries[0].class));
}

#[test]
fn duration_formats() {
    assert_eq!(parse_duration("0:03:21.000"), 201);
    assert_eq!(parse_duration("1:00:00"), 3600);
    assert_eq!(parse_duration("05:30"), 330);
    assert_eq!(parse_duration(""), 0);
}

#[test]
fn resolve_url_variants() {
    let base = "http://192.168.1.8:8200/description.xml";
    assert_eq!(
        resolve_url(base, "/MediaItems/12.mp3"),
        "http://192.168.1.8:8200/MediaItems/12.mp3"
    );
    assert_eq!(
        resolve_url(base, "MediaItems/12.mp3"),
        "http://192.168.1.8:8200/MediaItems/12.mp3"
    );
    assert_eq!(
        resolve_url(base, "http://other/MediaItems/1.mp3"),
        "http://other/MediaItems/1.mp3"
    );
}

/// 真实网络测试（需局域网内有 DLNA 服务器，手动跑）：
/// `cargo test --bin xiaozhi-server-rust dlna_discover_real_network -- --ignored --nocapture`
#[tokio::test]
#[ignore = "需要局域网内存在 DLNA 服务器，仅手动验证"]
async fn dlna_discover_real_network() {
    let servers = super::discover(Duration::from_secs(4)).await;
    println!("发现 {} 台 DLNA 媒体服务器：", servers.len());
    for s in &servers {
        println!("  - {} @ {}", s.name, s.location);
    }
    for s in &servers {
        match super::browse(&s.location).await {
            Ok(tracks) => {
                println!("  {}：{} 个曲目（前 3 个）", s.name, tracks.len());
                for t in tracks.iter().take(3) {
                    println!("    · {} / {} [{}s]", t.title, t.artist, t.duration);
                }
            }
            Err(e) => println!("  {}：浏览失败 {e}", s.name),
        }
    }
}
