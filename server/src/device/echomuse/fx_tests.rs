use super::fx::Fx;
use crate::device::echomuse::ControlOut;

/// 取出已下发指令的 type 序列（断言接线顺序用）。
fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ControlOut>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(ControlOut::Json(json)) = rx.try_recv().ok() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            out.push(v["type"].as_str().unwrap_or("").to_string());
        }
    }
    out
}

fn bound() -> (Fx, tokio::sync::mpsc::UnboundedReceiver<ControlOut>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (Fx::bound(tx), rx)
}

#[test]
fn anim_specs_carry_pattern_colors_and_dead_man_ttl() {
    let (fx, mut rx) = bound();
    assert!(fx.listening());
    assert!(fx.thinking());
    assert!(fx.speaking());
    assert!(fx.nospeech());
    let expected = [("solid", 30u64), ("spin", 10), ("meter", 120), ("pulse", 1)];
    for pattern_ttl in expected {
        let ControlOut::Json(json) = rx.try_recv().unwrap() else {
            panic!("指令缺失")
        };
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "led_anim");
        assert_eq!(v["anim"]["pattern"], pattern_ttl.0);
        assert_eq!(v["anim"]["ttlSec"], pattern_ttl.1);
        assert!(
            v["anim"]["ttlSec"].as_u64().unwrap() > 0,
            "死信开关必须为正，环才不会永远转下去"
        );
    }
}

#[test]
fn listening_anim_marks_the_listening_ring() {
    let (fx, mut rx) = bound();
    fx.listening();
    if let Some(ControlOut::Json(json)) = rx.try_recv().ok() {
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["anim"]["pattern"], "solid");
        assert_eq!(v["anim"]["listening"], true, "方向 overlay 依赖该标记");
        assert_eq!(v["anim"]["ttlSec"], 30);
        assert_eq!(v["anim"]["colors"][0], serde_json::json!([0, 180, 0]));
    } else {
        panic!("listening 未下发");
    }
}

#[test]
fn off_paints_twelve_dark_leds_and_cue_and_flush_shapes() {
    let (fx, mut rx) = bound();
    assert!(fx.off());
    assert!(fx.wake_cue());
    assert!(fx.speaker_flush());
    let mut kinds = sent(&mut rx);
    assert_eq!(
        kinds.drain(..3).collect::<Vec<_>>(),
        vec!["leds", "play_cue", "speaker_flush"]
    );
}

#[test]
fn unbound_fx_fails_quietly() {
    let fx = Fx::default();
    assert!(!fx.listening());
    assert!(!fx.off());
}
