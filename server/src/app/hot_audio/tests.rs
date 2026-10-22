//! [`super`]（持久化热句音频库）的回归测试。
//!
//! 重点不在"能存能取"，而在**失效语义与裸文件契约**：文件必须是可直接播放的裸 PCM
//!（无任何容器字节），文件名按内容寻址（换了音色/引擎/文本一律 miss，下行参数不参与
//! 磁盘身份），坏文件被安全丢弃，凭据绝不落盘，容量有界，旧格式文件绝不读取也不自动
//! 清理。这些错了不会报错，只会"播出别的音色的声音"或"磁盘慢慢涨"，所以每条都有
//! 独立断言。

use std::path::PathBuf;

use super::*;

/// 测试用源采样率（磁盘 PCM 的原样率）：身份与内存键都必须带上它。
const SRC_SR: u32 = 16_000;

/// 独立的临时目录（并发跑测试不会互相踩）。
fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xz-hot-{}", uuid::Uuid::new_v4()))
}

/// 默认配置下的键（TTS 引擎为内置默认）。磁盘身份**不含**下行参数。
fn key_of(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&Config::default(), text, SRC_SR)
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
fn 落盘的就是裸_pcm_无任何容器字节() {
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    let p = pcm(64);
    assert!(store.put(&k, &p));

    // 文件扩展名 .pcm，文件字节与写入载荷**逐字节一致**——外部工具可直接播放。
    let path = dir.join(format!("{:016x}.pcm", k.file_id()));
    assert_eq!(std::fs::read(&path).unwrap(), p, "不得有任何头部/校验尾");

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
    let path = dir.join(format!("{:016x}.pcm", k.file_id()));

    // 1) 同名垃圾文件（外部误放/写坏）：读不得命中，且删除避免长期占位
    std::fs::write(&path, b"garbage-not-pcm").unwrap();
    assert!(store.get(&k).is_none());
    assert!(!path.exists(), "作废文件应被删除");

    // 2) 奇数字节（非整 S16LE 样本）同样拒绝并清除
    std::fs::write(&path, [0u8, 0, 1]).unwrap();
    assert!(store.get(&k).is_none());
    assert!(!path.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 拒绝非法载荷() {
    let store = HotAudioStore::new(Some(tmp_dir()));
    let k = key_of("我在");
    assert!(!store.put(&k, &[]), "空音频不入库");
    assert!(!store.put(&k, &[0, 0, 1]), "奇数字节不是完整 S16LE 样本");
    assert!(
        !store.put(&k, &vec![0u8; MAX_ENTRY_BYTES + 2]),
        "超上限不入库"
    );
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
    let k = HotAudioKey::from_session(&live, "我在", SRC_SR);
    let expect = TtsCache::make_key(
        &crate::plugins::registry::signature_of(&live, crate::plugins::registry::Capability::Tts),
        live.tts.speaker,
        live.tts.speed,
        SRC_SR,
        fmt,
        "我在",
    );
    assert_eq!(k.mem_key(fmt), expect);

    // 换音色 → 内存键也必须变（否则会命中上一个音色的音频）
    let mut other_live = Config::default();
    other_live.tts.speaker = live.tts.speaker + 1;
    let k2 = HotAudioKey::from_session(&other_live, "我在", SRC_SR);
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

    let raw = std::fs::read(dir.join(format!("{:016x}.pcm", k.file_id()))).unwrap();
    let hay = String::from_utf8_lossy(&raw);
    assert!(
        !hay.contains("SUPER-SECRET-KEY"),
        "引擎签名（含密钥）绝不能明文落盘"
    );
    // 但身份仍然有效：同一签名能命中，换签名即 miss
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
        std::fs::write(dir.join(format!("{:016x}.pcm", key.file_id())), pcm(64)).unwrap();
    }
    assert!(store.put(&key_of("触发回收"), &pcm(64)));
    let n = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "pcm"))
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
fn 旧格式遗留不读取也不清理() {
    // `.hot` 容器 / `managed.json` 是历史格式：磁盘身份只按 `<hash>.pcm` 寻址，
    // 它们永远不会被读取，也**不做自动删除**（由使用者手动处理）。
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    std::fs::create_dir_all(&dir).unwrap();
    let legacy_hot = dir.join("0123456789abcdef.hot");
    let legacy_json = dir.join("managed.json");
    std::fs::write(&legacy_hot, b"XZHOTAU1-legacy").unwrap();
    std::fs::write(&legacy_json, b"{}").unwrap();

    assert!(store.get(&key_of("我在")).is_none(), "新键不得命中旧文件");
    assert!(legacy_hot.exists(), "旧格式文件不做自动清理");
    assert!(legacy_json.exists(), "登记表不做自动清理");

    // 放一个真缓存条目再触发淘汰：旧格式不计入 `.pcm` 容量口径。
    assert!(store.put(&key_of("触发"), &pcm(64)));
    assert!(legacy_hot.exists());

    let _ = std::fs::remove_dir_all(&dir);
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
    // 裸 .pcm 起的身份投影 =
    //   PCM_FORMAT_VERSION | FNV(签名) | speaker | speed bits | tts_sr | FNV(文本)。
    // 改任何一项打包口径都会让既有部署的缓存全部失效，必须显式换版本号。
    // v2：修正「引擎实为 16k 却被标成 24k」后弃用全部旧成品（旧文件内容与声明不符，
    //     继续命中会保持 1.5× 提速且重启不自愈），故整体换名强制重新合成。
    // v3：改为引擎原样落盘（不再统一重采样到 24k），并让源采样率参与身份。
    let key = HotAudioKey {
        engine_sig: "sig".into(),
        speaker: 0,
        speed: 1.0,
        tts_sr: SRC_SR,
        text: "我在".into(),
    };
    assert_eq!(key.file_id(), 0x522d_bafd_3eea_39a0);
}

#[test]
fn 落盘与读回逐字节一致() {
    // 「引擎给什么就存什么」：i16 源样本 → f32 → 落盘 → 读回，必须逐字节相同。
    // 覆盖正负极值：32768 标度下 f32 运算精确，任何 1 LSB 偏差都说明标度选错（32767）。
    let src: Vec<i16> = vec![0, 1, -1, 1000, -1000, 32_767, -32_768, 12_345, -23_456];
    let bytes: Vec<u8> = src.iter().flat_map(|v| v.to_le_bytes()).collect();
    let back = f32_to_pcm16_bytes(&pcm_bytes_to_f32(&bytes));
    assert_eq!(back, bytes, "落盘必须原样保留引擎字节（1 LSB 都不许丢）");
}

#[test]
fn 总字节上限也会回收() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    // sparse 文件只占逻辑大小，验证字节上限不需要真正写 512MB。
    for i in 0..2 {
        let path = dir.join(format!("oversize{i}.pcm"));
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

#[test]
fn 淘汰只看_pcm_不碰其他文件() {
    let dir = tmp_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let other = dir.join("managed.json");
    std::fs::write(&other, b"{}").unwrap();
    let path = dir.join("oversize.pcm");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_TOTAL_BYTES + 1)
        .unwrap();
    evict_overflow(&dir);
    assert!(!path.exists(), "超限 .pcm 被淘汰");
    assert!(other.exists(), "非 .pcm 文件不在淘汰职责内");

    let _ = std::fs::remove_dir_all(&dir);
}
