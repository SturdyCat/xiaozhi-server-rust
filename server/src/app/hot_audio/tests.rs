//! [`super`]（持久化热句音频库）的回归测试。
//!
//! 重点不在"能存能取"，而在**失效语义**：文件必须自校验（换了音色/引擎/文本一律 miss，
//! 下行参数**不**参与磁盘身份），坏文件与旧格式必须被安全丢弃，凭据绝不落盘，容量有界。
//! 这些错了不会报错，只会"播出别的音色的声音"或"磁盘慢慢涨"，所以每条都有独立断言。

use std::path::{Path, PathBuf};

use super::*;

/// 独立的临时目录（并发跑测试不会互相踩）。
fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xz-hot-{}", uuid::Uuid::new_v4()))
}

/// 默认配置下的键（TTS 引擎为内置默认）。磁盘身份**不含**下行参数。
fn key_of(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&Config::default(), text)
}

/// 造 `n` 字节可区分的 PCM 负载（保持偶数字节 = 整 S16LE 样本）。
fn pcm(n: usize) -> Vec<u8> {
    let n = n.max(2) & !1;
    (0..n).map(|i| (i % 251) as u8).collect()
}

#[test]
fn 存取往返_内容不变() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    let p = pcm(64);

    assert!(store.put(&k, &p), "应落盘");
    let got = store.get(&k).expect("应命中");
    assert_eq!(got, p, "PCM 字节必须逐字节一致");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 跨实例可读_模拟进程重启() {
    let dir = tmp_dir();
    let k = key_of("我在");
    let p = pcm(48);
    // 实例 A 写（随后丢弃 = 进程退出，内存缓存消失）
    assert!(HotAudioStore::new(Some(dir.clone())).put(&k, &p));
    // 实例 B 读：只有磁盘这一条路
    let got = HotAudioStore::new(Some(dir.clone()))
        .get(&k)
        .expect("重启后应命中");
    assert_eq!(got, p);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换文本即_miss() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    assert!(store.put(&key_of("我在"), &pcm(32)));
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
    assert!(store.put(&a, &pcm(32)));
    assert!(store.get(&b).is_none(), "换音色（speaker）必须 miss");

    // 语速同样参与键
    a.speed = 1.25;
    b.speed = 1.0;
    assert!(store.put(&a, &pcm(32)));
    assert!(store.get(&b).is_none(), "换语速必须 miss");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 坏文件被丢弃且不影响会话() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{:016x}.hot", k.file_id()));

    // 1) 魔数错（含旧 Opus 格式文件：v3 起魔数/版本都不同，失配即清）
    std::fs::write(&path, b"NOTMAGIC................").unwrap();
    assert!(store.get(&k).is_none());
    assert!(!path.exists(), "作废文件应被删除，避免长期占位");

    // 2) 截断（长度字段越界）
    std::fs::write(&path, encode(&k, &pcm(48))[..40].to_vec()).unwrap();
    assert!(store.get(&k).is_none());

    // 3) PCM 长度字段被改成超大值（防御性上限）
    let mut raw = encode(&k, &pcm(48));
    raw[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&path, &raw).unwrap();
    assert!(store.get(&k).is_none(), "超上限 PCM 必须拒绝而不是尝试分配");

    // 4) 头部格式常量被改（存储采样率字段 ≠ 24k）
    let mut raw = encode(&k, &pcm(48));
    raw[28] ^= 1;
    std::fs::write(&path, &raw).unwrap();
    assert!(store.get(&k).is_none(), "存储采样率必须与格式常量一致");
    assert!(!path.exists());

    // 5) 空/奇数字节 PCM：不写
    assert!(!store.put(&k, &[]), "空音频不入库");
    assert!(!store.put(&k, &[0, 0, 1]), "奇数字节不是完整 S16LE 样本");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 未配置目录时纯内存降级() {
    let store = HotAudioStore::new(None);
    assert!(!store.is_persistent());
    let k = key_of("我在");
    assert!(!store.put(&k, &pcm(32)), "无目录 → 不落盘");
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
    // 与内存键是两套（前者跨版本稳定，后者与预取同口径且含下行参数）
    assert_ne!(
        base.file_id(),
        base.mem_key(crate::app::audio::media::AudioFormat::new(
            crate::app::audio::media::AudioCodec::Opus,
            16_000,
            60,
        ))
    );
}

#[test]
fn mem_key_与_tts_cache_同口径() {
    use crate::app::audio::media::{AudioCodec, AudioFormat};
    // 这条是"暖进内存缓存后能命中"的前提：两处必须算出同一个键。
    let live = Config::default();
    let fmt = AudioFormat::new(AudioCodec::Opus, 24_000, 60);
    let k = HotAudioKey::from_session(&live, "我在");
    let expect = TtsCache::make_key(
        &crate::plugins::registry::signature_of(&live, crate::plugins::registry::Capability::Tts),
        live.tts.speaker,
        live.tts.speed,
        fmt,
        "我在",
    );
    assert_eq!(k.mem_key(fmt), expect);

    // 换音色 → 内存键也必须变（否则会命中上一个音色的音频）
    let mut other_live = Config::default();
    other_live.tts.speaker = live.tts.speaker + 1;
    let k2 = HotAudioKey::from_session(&other_live, "我在");
    assert_ne!(k.mem_key(fmt), k2.mem_key(fmt));

    // 下行格式变（采样率/帧长/编解码档）→ 内存键变（帧按会话格式编码），但磁盘身份不受影响
    let fmt2 = AudioFormat::new(AudioCodec::Opus, 16_000, 60);
    let fmt3 = AudioFormat::new(AudioCodec::PcmS16Le, 24_000, 60);
    assert_ne!(k.mem_key(fmt), k.mem_key(fmt2));
    assert_ne!(k.mem_key(fmt), k.mem_key(fmt3));
    let id = k.file_id();
    assert_eq!(id, k.file_id(), "磁盘身份与下行格式无关（键里就没有它们）");
}

#[test]
fn 凭据不落盘() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let mut k = key_of("我在");
    k.engine_sig = "xfyun|api_key=SUPER-SECRET-KEY|voice=x6".to_string();
    assert!(store.put(&k, &pcm(32)));

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
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..(MAX_FILES + 5) {
        let key = key_of(&format!("句{i}"));
        std::fs::write(
            dir.join(format!("{:016x}.hot", key.file_id())),
            encode(&key, &pcm(64)),
        )
        .unwrap();
    }
    assert!(store.put(&key_of("触发回收"), &pcm(64)));
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

#[test]
fn 旧帧格式文件被拒绝并清除() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{:016x}.hot", k.file_id()));
    // 旧 v2 文件：旧魔数 + 帧表正文。v3 起（PCM）不再兼容——读不了就清，
    // 首次使用重新合成一次即可。
    let mut raw = vec![0u8; 96];
    raw[0..8].copy_from_slice(b"XZHOTAU1");
    raw[8..12].copy_from_slice(&2u32.to_le_bytes());
    std::fs::write(&path, &raw).unwrap();
    assert!(store.get(&k).is_none(), "旧 Opus 格式不得命中");
    assert!(!path.exists(), "旧格式文件应被清除，避免长期占位");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 校验和拒绝负载损坏和尾随垃圾() {
    let key = key_of("普通生产文本");
    let mut raw = encode(&key, &pcm(64));
    let last = raw.len() - 9;
    raw[last] ^= 1;
    assert!(decode(&raw, &key.header()).is_err());
    let mut raw = encode(&key, &pcm(64));
    raw.push(0);
    assert!(decode(&raw, &key.header()).is_err());
}

#[test]
fn 拒绝超限句子与声明不一致的正文() {
    let store = HotAudioStore::new(Some(tmp_dir()));
    let key = key_of("文本");
    assert!(!store.put(&key, &vec![0u8; MAX_ENTRY_BYTES + 2]));
    let mut raw = encode(&key, &pcm(64));
    raw[44..48].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&raw, &key.header()).is_err());
    // 头部声明的长度与正文实际长度不一致
    let mut raw = encode(&key, &pcm(64));
    raw[44..48].copy_from_slice(&1024u32.to_le_bytes());
    assert!(decode(&raw, &key.header()).is_err());
}

#[test]
fn 目录跟随配置来源或记忆库() {
    assert_eq!(
        cache_dir(Some("/volume/config.toml"), "/other/memory.db"),
        PathBuf::from("/volume/hot-audio")
    );
    assert_eq!(
        cache_dir(None, "/volume/memory.db"),
        PathBuf::from("/volume/hot-audio")
    );
    assert_eq!(cache_dir(None, "memory.db"), PathBuf::from("./hot-audio"));
    assert_eq!(
        cache_dir(Some("config.toml"), "/other/memory.db"),
        PathBuf::from("./hot-audio")
    );
}

#[test]
fn 文件名投影保持稳定() {
    // v3（PCM 24k）：身份 = 签名/音色/语速/文本；sr=24000、frame_ms=0 是格式常量。
    // 改任何一项打包口径都会让既有部署的缓存全部失效，必须显式换版本号。
    let key = HotAudioKey {
        engine_sig: "sig".into(),
        speaker: 0,
        speed: 1.0,
        text: "我在".into(),
    };
    assert_eq!(key.file_id(), 0x64c2_029c_5e07_3542);
}

#[test]
fn 总字节上限也会回收() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // sparse 文件只占逻辑大小，验证字节上限不需要真正写 512MB。
    for i in 0..2 {
        let path = dir.join(format!("oversize{i}.hot"));
        std::fs::File::create(path)
            .unwrap()
            .set_len(MAX_TOTAL_BYTES)
            .unwrap();
    }
    evict_overflow(&dir);
    let bytes: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(bytes <= MAX_TOTAL_BYTES);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- 管理槽位（managed.json）：启动校验与陈旧清理 ----

fn managed(slot: &str, text: &str) -> ManagedText {
    ManagedText {
        slot: slot.to_string(),
        text: text.to_string(),
    }
}

fn hot_path(dir: &Path, k: &HotAudioKey) -> PathBuf {
    dir.join(format!("{:016x}.hot", k.file_id()))
}

#[test]
fn put_managed_登记且跨实例存续() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    assert!(store.put_managed(SLOT_WAKEUP_GREETING, &k, &pcm(64)));

    // 登记表：含文件 id；不含文本明文（凭据口径一致）
    let raw = std::fs::read(dir.join("managed.json")).unwrap();
    let hay = String::from_utf8_lossy(&raw);
    assert!(hay.contains(&format!("{:016x}", k.file_id())));
    assert!(!hay.contains("我在"), "登记表不得含文本明文");

    // 重复登记幂等
    assert!(store.put_managed(SLOT_WAKEUP_GREETING, &k, &pcm(64)));
    let reg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("managed.json")).unwrap()).unwrap();
    assert_eq!(
        reg["slots"][SLOT_WAKEUP_GREETING].as_array().unwrap().len(),
        1,
        "同一条目不得重复登记"
    );

    // 新实例（= 重启）按同一清单校验：有效保留，文件不动
    let summary = HotAudioStore::new(Some(dir.clone()))
        .sync_managed(&[managed(SLOT_WAKEUP_GREETING, "我在")]);
    assert!(summary.contains("1 条有效"), "{summary}");
    assert!(hot_path(&dir, &k).exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 文案改动_启动清理旧音频() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let old = key_of("我已连接，喊我的唤醒词就能开始对话。");
    let new = key_of("我上线啦，喊我就能聊天。");
    assert!(store.put_managed(SLOT_ECHOMUSE_WELCOME, &old, &pcm(64)));

    // 文案已改、新音频尚未合成：旧文件在启动校验时删除
    let summary = store.sync_managed(&[managed(SLOT_ECHOMUSE_WELCOME, "我上线啦，喊我就能聊天。")]);
    assert!(summary.contains("1 条过时"), "{summary}");
    assert!(!hot_path(&dir, &old).exists(), "旧文案音频必须删除");
    // 新文案随后首次使用 → 重新合成落盘并重新登记
    assert!(store.put_managed(SLOT_ECHOMUSE_WELCOME, &new, &pcm(64)));
    assert!(hot_path(&dir, &new).exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 槽位移除_整槽清理() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    assert!(store.put_managed(SLOT_WAKEUP_GREETING, &k, &pcm(64)));
    // 问候语清空 → 清单中无该槽位 → 旧音频删除
    let summary = store.sync_managed(&[]);
    assert!(summary.contains("1 条过时"), "{summary}");
    assert!(!hot_path(&dir, &k).exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 多句池_只清改掉的那句() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let a = key_of("我在");
    let b = key_of("在呢");
    let c = key_of("来啦");
    assert!(store.put_managed(SLOT_WAKEUP_GREETING, &a, &pcm(64)));
    assert!(store.put_managed(SLOT_WAKEUP_GREETING, &b, &pcm(64)));
    // 池子改成「在呢 / 来啦」：「我在」过时；「来啦」尚未合成、不该有文件
    let summary = store.sync_managed(&[
        managed(SLOT_WAKEUP_GREETING, "在呢"),
        managed(SLOT_WAKEUP_GREETING, "来啦"),
    ]);
    assert!(summary.contains("1 条有效"), "{summary}");
    assert!(summary.contains("1 条过时"), "{summary}");
    assert!(!hot_path(&dir, &a).exists());
    assert!(hot_path(&dir, &b).exists());
    assert!(!hot_path(&dir, &c).exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 损坏成品_启动删除待重合成() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("好的");
    assert!(store.put_managed(SLOT_SKILL_REPLY, &k, &pcm(64)));
    let path = hot_path(&dir, &k);
    let mut raw = std::fs::read(&path).unwrap();
    let last = raw.len() - 9;
    raw[last] ^= 1; // 破坏校验和
    std::fs::write(&path, &raw).unwrap();

    let summary = store.sync_managed(&[managed(SLOT_SKILL_REPLY, "好的")]);
    assert!(summary.contains("1 条损坏"), "{summary}");
    assert!(!path.exists(), "损坏成品必须删除，下次使用重新合成");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 非管理文件_校验不碰() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("一句普通的LLM回复");
    assert!(store.put(&k, &pcm(64))); // 普通生产路径：无登记
    store.sync_managed(&[managed(SLOT_ECHOMUSE_WELCOME, "欢迎")]);
    assert!(
        hot_path(&dir, &k).exists(),
        "未登记文件不归启动校验管（仍由溢出淘汰）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 未持久化时管理路径为无操作() {
    let store = HotAudioStore::new(None);
    assert!(!store.put_managed(SLOT_ECHOMUSE_WELCOME, &key_of("我在"), &pcm(64)));
    let summary = store.sync_managed(&[managed(SLOT_ECHOMUSE_WELCOME, "我在")]);
    assert!(summary.contains("跳过"), "{summary}");
}

mod media;
