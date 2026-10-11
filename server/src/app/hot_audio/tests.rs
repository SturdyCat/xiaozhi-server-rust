//! [`super`]（持久化热句音频库）的回归测试。
//!
//! 重点不在"能存能取"，而在**失效语义**：文件必须自校验（换了音色/采样率/帧长/文本一律
//! miss），坏文件必须被安全丢弃，凭据绝不落盘，容量有界。这些错了不会报错，只会"播出
//! 别的音色的声音"或"磁盘慢慢涨"，所以每条都有独立断言。

use std::path::PathBuf;

use super::*;

/// 独立的临时目录（并发跑测试不会互相踩）。
fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xz-hot-{}", uuid::Uuid::new_v4()))
}

/// 默认配置下的键（TTS 引擎为内置默认）。
fn key_of(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&Config::default(), 24_000, 60, text)
}

/// 造 `n` 帧可区分的负载。
fn frames(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; 8]).collect()
}

#[test]
fn 存取往返_帧序与内容不变() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    let f = frames(5);

    assert!(store.put(&k, &f), "应落盘");
    let got = store.get(&k).expect("应命中");
    assert_eq!(got, f, "帧序与内容必须逐字节一致");
    assert_eq!(got.len(), 5);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 跨实例可读_模拟进程重启() {
    let dir = tmp_dir();
    let k = key_of("我在");
    let f = frames(3);
    // 实例 A 写（随后丢弃 = 进程退出，内存缓存消失）
    assert!(HotAudioStore::new(Some(dir.clone())).put(&k, &f));
    // 实例 B 读：只有磁盘这一条路
    let got = HotAudioStore::new(Some(dir.clone())).get(&k).expect("重启后应命中");
    assert_eq!(got, f);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换文本即_miss() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    assert!(store.put(&key_of("我在"), &frames(2)));
    assert!(store.get(&key_of("在呢")).is_none(), "不同文本不得串用");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换音色即_miss() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let mut a = key_of("我在");
    let mut b = key_of("我在");
    b.speaker = a.speaker + 1;
    assert!(store.put(&a, &frames(2)));
    assert!(store.get(&b).is_none(), "换音色（speaker）必须 miss");

    // 语速同样参与键
    a.speed = 1.25;
    b.speed = 1.0;
    assert!(store.put(&a, &frames(2)));
    assert!(store.get(&b).is_none(), "换语速必须 miss");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换采样率或帧长即_miss() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let a = HotAudioKey::from_session(&Config::default(), 24_000, 60, "我在");
    let b = HotAudioKey::from_session(&Config::default(), 16_000, 60, "我在");
    let c = HotAudioKey::from_session(&Config::default(), 24_000, 20, "我在");
    assert!(store.put(&a, &frames(2)));
    assert!(store.get(&b).is_none(), "下行采样率不同必须 miss");
    assert!(store.get(&c).is_none(), "帧长不同必须 miss");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 坏文件被丢弃且不影响会话() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{:016x}.hot", k.file_id()));

    // 1) 魔数错
    std::fs::write(&path, b"NOTMAGIC................").unwrap();
    assert!(store.get(&k).is_none());
    assert!(!path.exists(), "作废文件应被删除，避免长期占位");

    // 2) 截断（长度字段越界）
    std::fs::write(&path, encode(&k, &frames(3))[..40].to_vec()).unwrap();
    assert!(store.get(&k).is_none());

    // 3) 帧长度字段被改成超大值（防御性上限）
    let mut raw = encode(&k, &frames(1));
    raw[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&path, &raw).unwrap();
    assert!(store.get(&k).is_none(), "超上限帧长必须拒绝而不是尝试分配");

    // 4) 空帧序列：不写
    assert!(!store.put(&k, &[]), "空音频不入库");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 未配置目录时纯内存降级() {
    let store = HotAudioStore::new(None);
    assert!(!store.is_persistent());
    let k = key_of("我在");
    assert!(!store.put(&k, &frames(2)), "无目录 → 不落盘");
    assert!(store.get(&k).is_none(), "无目录 → 不命中");
}

#[test]
fn file_id_稳定且区分输入() {
    // 稳定性：同一输入两次构造必须同 id（否则跨进程永远读不到自己写的文件）
    assert_eq!(key_of("我在").file_id(), key_of("我在").file_id());
    // 区分性：任一维度变化都必须换 id
    let base = key_of("我在");
    let mut other = key_of("我在");
    other.speed = 1.5;
    assert_ne!(base.file_id(), other.file_id());
    assert_ne!(base.file_id(), key_of("在呢").file_id());
    // 与内存键是两套（前者跨版本稳定，后者与预取同口径）
    assert_ne!(base.file_id(), base.mem_key());
}

#[test]
fn mem_key_与_tts_cache_同口径() {
    // 这条是"暖进内存缓存后能命中"的前提：两处必须算出同一个键。
    let live = Config::default();
    let k = HotAudioKey::from_session(&live, 24_000, 60, "我在");
    let expect = TtsCache::make_key(
        &crate::plugins::registry::signature_of(&live, crate::plugins::registry::Capability::Tts),
        live.tts.speaker,
        live.tts.speed,
        24_000,
        60,
        "我在",
    );
    assert_eq!(k.mem_key(), expect);

    // 换音色 → 内存键也必须变（否则会命中上一个音色的音频）
    let mut other_live = Config::default();
    other_live.tts.speaker = live.tts.speaker + 1;
    let k2 = HotAudioKey::from_session(&other_live, 24_000, 60, "我在");
    assert_ne!(k.mem_key(), k2.mem_key());
}

#[test]
fn 凭据不落盘() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let mut k = key_of("我在");
    k.engine_sig = "xfyun|api_key=SUPER-SECRET-KEY|voice=x6".to_string();
    assert!(store.put(&k, &frames(2)));

    let raw = std::fs::read(dir.join(format!("{:016x}.hot", k.file_id()))).unwrap();
    let hay = String::from_utf8_lossy(&raw);
    assert!(
        !hay.contains("SUPER-SECRET-KEY"),
        "引擎签名（含密钥）绝不能明文落盘"
    );
    // 但校验仍然有效：同一签名能命中，换签名即 miss
    assert!(store.get(&k).is_some());
    k.engine_sig = "xfyun|api_key=OTHER|voice=x6".to_string();
    assert!(store.get(&k).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 超出上限被淘汰() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    for i in 0..(MAX_FILES + 5) {
        assert!(store.put(&key_of(&format!("句{i}")), &frames(1)));
    }
    let n = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "hot"))
        .count();
    assert!(n <= MAX_FILES, "文件数必须被钉在上限内，实际 {n}");
    // 无残留临时文件
    let tmp = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
        .count();
    assert_eq!(tmp, 0, "临时文件必须已被 rename 消费");

    let _ = std::fs::remove_dir_all(&dir);
}
