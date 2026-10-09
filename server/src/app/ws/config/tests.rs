use super::*;

mod impact;
mod secrets;

/// 完整配置 fixture（**刻意写成 P4 之前的旧位置形态**）：`[tts] backend`/`model`/…、
/// `[memory] backend`。它同时就是「旧 config.toml 原样可读」这条 P4 验收的回归用例
/// （见 [`legacy_toml_fixture_migrates_to_convention`]）。
fn raw_legacy_toml() -> &'static str {
    r#"
[server]
port = 8123
expected_token = "tk"
worker_threads = 3
admin_dir = "/app/web"

[audio]
downlink_sample_rate = 22050
downlink_frame_duration_ms = 40
channels = 2
binary_protocol_version = 3
downlink_lead_ms = 321

[asr]
model = "/m/asr.onnx"
tokens = "/m/asr.txt"
language = "zh"
use_itn = false
num_threads = 3
provider = "cpu"

[vad]
model = "/m/vad.onnx"
threshold = 0.42
min_silence_duration = 0.31
min_speech_duration = 0.12

[tts]
backend = "sherpa"
model = "/m/k.onnx"
voices = "/m/voices.bin"
tokens = "/m/tokens.txt"
data_dir = "/m/data"
dict_dir = "/m/dict"
lexicon = "/m/a.txt,/m/b.txt"
lang = "en"
speaker = 3
speed = 1.25
num_threads = 2
cache_entries = 77

[tts.xfyun]
app_id = "app"
api_key = "key"
api_secret = "secret"
voice = "xiaoyan"

[llm]
api_base = "https://x/v1/responses"
api_key = "sk"
model = "m1"
system_prompt = "p"
max_history = 7
temperature = 0.3
stream = false

[aiui]
enabled = true
appid = "aid"
api_key = "ak"
api_secret = "as"
scene = "main"
sn_prefix = "sn"
voice = "v"
speed = 60
volume = 70
pitch = 40
prompt = "hi"
pace_ms = 12

[soul]
enabled = true
name = "小狐"
self_intro = "我是住在音箱里的小狐狸。"
form = "小狐狸"
age_feel = "十来岁"
worldview = "世界很大，慢慢看"
backstory = "从山里来的"
relationship_origin = "你在雨夜把我捡回家"
traits = ["好奇：遇到新话题会追问一句"]
values = ["不编造事实"]
boundaries = ["不聊血腥细节"]
tone = "轻快"
colloquial = false
address_user = "主人"
catchphrases = ["嗯哼"]
emoji = true
max_sentences = 3
max_chars = 60
scenario = "书房里"
device_hint = "客厅音箱"
examples = ["用户：今天几度？ / 小狐：我看不到天气"]

[memory]
enabled = true
backend = "graph"
db_path = "/data/mem.db"
db_busy_timeout_ms = 4321
recall_enabled = false
extraction_enabled = false
fresh_turn_count = 3
recall_max_nodes = 4
recall_max_tokens = 250
recall_budget_ms = 111
maintenance_interval = 9
inject_position = "before_history"
quarantine_max_attempts = 2
scope_prefix = "xiaozhi-dev"

[memory.retention]
keep = "recent"
recent_turns = 7
retention_days = 30
dry_run = true

[memory.extractor]
api_base = "https://ex/v1/responses"
api_key = "ex-key"
model = "small"
temperature = 0.3
"#
}

/// 完整配置 fixture（**每个字段都设为非默认值**），作为「往返不丢字段」的基准。
/// ⚠️ 新增配置字段时必须在此补上——这是防「管理页保存一次就静默重置手工配置」
/// （docs/plugin-architecture-unification.md §1.7 bug-1）的测试护栏。
///
/// 与 [`raw_legacy_toml`] 的关系：同一份 TOML，只是经过 `normalize()`（= 真实加载路径）。
/// 因此本函数的返回值是**规范形状**（`[tts.kokoro]`、`engine` 键已补全）。
fn full_test_config() -> Config {
    let mut cfg: Config = toml::from_str(raw_legacy_toml())
        .expect("测试配置 TOML 应能解析（新增字段后请同步本 fixture）");
    cfg.normalize();
    cfg
}

/// **P4 验收**：旧 config.toml（`[tts] backend/model/…`）原样可读，且规范化后
/// ① 落到规范位置；② 序列化只写新结构（旧键消失）。
#[test]
fn legacy_toml_fixture_migrates_to_convention() {
    let mut cfg: Config = toml::from_str(raw_legacy_toml()).unwrap();
    // 未规范化时：旧位置有值，新位置仍是默认值（说明"迁移"确实由 normalize 完成）
    assert_eq!(
        cfg.tts.kokoro.model,
        crate::plugins::tts::KokoroTtsConfig::default().model,
        "解析阶段不应擅自搬家"
    );
    assert_eq!(cfg.tts.engine_id(), "kokoro", "旧 backend=\"sherpa\" 映射为 kokoro");

    cfg.normalize();
    assert_eq!(cfg.tts.engine, "kokoro", "engine 键被补全为规范值");
    assert_eq!(cfg.tts.kokoro.model, "/m/k.onnx", "旧 [tts].model → [tts.kokoro].model");
    assert_eq!(cfg.tts.kokoro.lang, "en");
    assert_eq!(cfg.tts.kokoro.num_threads, 2);
    // 跨实现参数留在本体（位置不变，不参与搬家）
    assert_eq!(cfg.tts.speed, 1.25);
    assert_eq!(cfg.tts.speaker, 3);
    assert_eq!(cfg.memory.engine_id(), "graph", "旧 [memory].backend → engine");
    assert_eq!(cfg.asr.engine_id(), "sensevoice", "缺失 engine → 默认实现");
    assert_eq!(cfg.llm.engine_id(), "openai");

    // 保存形态：只写新结构
    let text = toml::to_string_pretty(&cfg).unwrap();
    assert!(text.contains("[tts.kokoro]"), "{text}");
    assert!(text.contains("engine = \"kokoro\""), "{text}");
    assert!(!text.contains("backend ="), "旧键不得写回: {text}");
    assert!(text.contains("engine = \"graph\""), "memory 也应写 engine: {text}");

    // 幂等：再规范化一次结果不变（load → normalize 会被调用多次）
    let mut again = cfg.clone();
    again.normalize();
    assert_eq!(
        serde_json::to_value(&again).unwrap(),
        serde_json::to_value(&cfg).unwrap(),
        "normalize 必须幂等"
    );
}

/// **新位置优先**：规范位置有值时不搬旧位置（同一文件里两处并存时以新位置为准）。
#[test]
fn new_position_wins_over_legacy() {
    let mut cfg: Config = toml::from_str(
        r#"
[tts]
engine = "kokoro"
model = "/old/model.onnx"

[tts.kokoro]
model = "/new/model.onnx"
"#,
    )
    .unwrap();
    cfg.normalize();
    assert_eq!(cfg.tts.kokoro.model, "/new/model.onnx");
}

/// `put_config` 的**真实**管线（此前是复刻，容易与生产代码漂移——已合并到 [`merge_client_patch`]）。
fn simulate_put(current: &Config, patch: serde_json::Value) -> Config {
    merge_client_patch(current, &patch).expect("合并结果应能反序列化为 Config")
}

// ---------- merge_patch：部分更新语义 ----------

#[test]
fn merge_patch_recurses_and_replaces_non_objects() {
    // 对象递归合并：只覆盖出现的键
    let mut base = serde_json::json!({"a": {"b": 1, "c": 2}});
    merge_patch(&mut base, &serde_json::json!({"a": {"b": 9}}));
    assert_eq!(base, serde_json::json!({"a": {"b": 9, "c": 2}}));

    // 基线不是对象 → 整体替换
    let mut base = serde_json::json!({"a": 1});
    merge_patch(&mut base, &serde_json::json!({"a": {"b": 2}}));
    assert_eq!(base, serde_json::json!({"a": {"b": 2}}));

    // 补丁不是对象 → 整体替换（随后由反序列化报精确类型错误）
    let mut base = serde_json::json!({"a": {"b": 1}});
    merge_patch(&mut base, &serde_json::json!(5));
    assert_eq!(base, serde_json::json!(5));
}

#[test]
fn merge_patch_explicit_null_clears_to_default() {
    let saved = simulate_put(
        &full_test_config(),
        serde_json::json!({"llm": {"api_key": null}, "audio": {"downlink_lead_ms": null}}),
    );
    assert_eq!(
        saved.llm.api_key, "",
        "显式 null = 清除 → 回落 serde 默认值"
    );
    assert_eq!(saved.audio.downlink_lead_ms, 240);
    // 同段其它字段不受影响
    assert_eq!(saved.llm.model, "m1");
    assert_eq!(saved.audio.downlink_sample_rate, 22050);
}

#[test]
fn merge_patch_absent_field_is_preserved() {
    let saved = simulate_put(
        &full_test_config(),
        serde_json::json!({"llm": {"model": "m2"}}),
    );
    assert_eq!(saved.llm.model, "m2", "出现的字段生效");
    assert_eq!(saved.llm.api_key, "sk", "未出现的字段保持原值");
    assert_eq!(saved.tts.cache_entries, 77);
    assert!(saved.aiui.enabled);
}

// ---------- bug-1 回归：旧客户端表单形状不再重置配置 ----------

/// 旧版管理页 `ConfigFormState.save()` 只 `put` 六个段，且
/// `audio` 漏 `downlink_lead_ms`、`tts` 漏 `cache_entries`、**完全没有 `[aiui]`**。
/// 修复前：保存一次就把手工配置的三处静默重置（AIUI 全链路被关掉）。
/// 修复后：未出现的字段保持不变；出现的字段照常生效。
#[test]
fn legacy_client_form_shape_no_longer_resets_omitted_sections() {
    let legacy_client_json = serde_json::json!({
        "server": {
            "port": 8000,
            "expected_token": "tk2",
            "worker_threads": 2,
            "admin_dir": "/app/web"
        },
        "audio": {
            "downlink_sample_rate": 24000,
            "downlink_frame_duration_ms": 60,
            "channels": 1,
            "binary_protocol_version": 1
        },
        "asr": {
            "model": "/m/asr.onnx",
            "tokens": "/m/asr.txt",
            "language": "zh",
            "use_itn": false,
            "num_threads": 3,
            "provider": "cpu"
        },
        "vad": {
            "model": "/m/vad.onnx",
            "threshold": 0.42,
            "min_silence_duration": 0.31,
            "min_speech_duration": 0.12
        },
        "tts": {
            "backend": "xfyun",
            "model": "/m/k2.onnx",
            "voices": "/m/voices.bin",
            "tokens": "/m/tokens.txt",
            "data_dir": "/m/data",
            "dict_dir": "/m/dict",
            "lexicon": "/m/a.txt,/m/b.txt",
            "lang": "en",
            "speed": 1.25,
            "num_threads": 2,
            "xfyun": {
                "app_id": "app",
                "api_key": "key",
                "api_secret": "secret",
                "voice": "xiaoyan"
            }
        },
        "llm": {
            "api_base": "https://x/v1/responses",
            "api_key": "sk",
            "model": "m1",
            "system_prompt": "p",
            "max_history": 7,
            "temperature": 0.3,
            "stream": false
        }
    });

    let saved = simulate_put(&full_test_config(), legacy_client_json);

    // 客户端未发送的三个字段：必须保持手工配置的值
    assert_eq!(
        saved.audio.downlink_lead_ms, 321,
        "audio.downlink_lead_ms 不应被重置"
    );
    assert_eq!(saved.tts.cache_entries, 77, "tts.cache_entries 不应被重置");
    assert!(saved.aiui.enabled, "[aiui] 不应被静默关闭");
    assert_eq!(saved.aiui.appid, "aid");
    assert_eq!(saved.aiui.api_secret, "as");

    // 客户端确实发送的字段：照常生效
    assert_eq!(saved.server.port, 8000);
    assert_eq!(saved.server.expected_token, "tk2");
    assert_eq!(saved.tts.engine, "xfyun", "旧键 backend 应被重写为 engine");
    // **旧客户端补丁写的是旧位置**（`tts.model`）：P4 的补丁重写必须让它仍然生效，
    // 而不是被"新位置优先"静默丢弃——这是 19 处扩展成本里最容易踩坏的一处。
    assert_eq!(
        saved.tts.kokoro.model, "/m/k2.onnx",
        "旧位置的 model 改动必须落到 [tts.kokoro]"
    );
    assert_eq!(
        saved.tts.kokoro.lang, "en",
        "补丁未提到的旧位置字段保持盘上值"
    );
}

// ---------- 全字段往返：逐字段断言，而不是只挑几个手写字段 ----------

/// 「GET 拿到的完整配置原样提交回来」必须**逐字段无损**（TOML 序列化等价）。
/// 手写形状的往返回归测不出字段遗漏；本测试以完整 `Config` 为基准，
/// 新增字段后若 fixture/序列化路径有问题会直接打红。
#[test]
fn full_config_roundtrip_is_lossless() {
    let original = full_test_config();
    let echoed = serde_json::to_value(&original).expect("Config → JSON");
    let saved = simulate_put(&original, echoed);

    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&original).unwrap(),
        "完整配置往返后应逐字段相等"
    );
    assert_eq!(
        toml::to_string_pretty(&saved).unwrap(),
        toml::to_string_pretty(&original).unwrap(),
        "写盘 TOML 也应等价（覆盖 serde 默认值路径）"
    );
}

/// P6 新增段（`[soul]`/`[memory]`）必须同样受"缺失 = 保持"保护：
/// 否则"管理页还认不得这个段"就会在保存一次后被重置（§1.7 bug-1 的同类风险）。
#[test]
fn new_p6_sections_are_not_reset_by_old_client_patch() {
    let original = full_test_config();
    let saved = simulate_put(&original, serde_json::json!({"llm": {"model": "m2"}}));
    assert_eq!(saved.llm.model, "m2");
    assert_eq!(saved.soul.name, "小狐", "人格段不该被未提及它的补丁重置");
    assert_eq!(saved.soul.max_sentences, 3);
    assert_eq!(saved.soul.examples.len(), 1);
    assert_eq!(saved.memory.db_path, "/data/mem.db");
    assert_eq!(saved.memory.recall_max_tokens, 250);
    assert_eq!(saved.memory.retention.keep, "recent");
    assert_eq!(saved.memory.extractor.model, "small");
    assert_eq!(saved.memory.inject_position, "before_history");
}

/// 部分更新合并后写盘 → 重新解析：未提及的段与提及的段都要保真。
#[test]
fn partial_update_survives_disk_roundtrip() {
    let dir = std::env::temp_dir().join(format!("xz_cfg_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");

    let merged = simulate_put(
        &full_test_config(),
        serde_json::json!({"llm": {"model": "m9"}, "tts": {"cache_entries": 5}}),
    );
    persist_config(path.to_str().unwrap(), &merged).unwrap();
    let loaded = Config::load(path.to_str().unwrap()).unwrap();

    assert_eq!(loaded.llm.model, "m9");
    assert_eq!(loaded.tts.cache_entries, 5);
    assert_eq!(
        loaded.audio.downlink_lead_ms, 321,
        "未提及的段在写盘往返后仍保真"
    );
    assert!(loaded.aiui.enabled);
    assert_eq!(loaded.aiui.pace_ms, 12);
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------- 既有回归 ----------

/// JSON 形状兼容：macApp 提交的 JSON 经「合并 → 反序列化」后类型都对得上。
#[test]
fn client_json_shape_parses_through_merge() {
    let cfg = simulate_put(
        &Config::default(),
        serde_json::json!({
            "server": {"port": 8000, "expected_token": "tk", "worker_threads": 2, "admin_dir": "/app/web"},
            "tts": {"backend": "xfyun", "xfyun": {"app_id": "app", "voice": "xiaoyan"}},
            "llm": {"api_base": "https://api.example.com/v1/responses", "model": "gpt-4o", "stream": true}
        }),
    );
    assert_eq!(cfg.server.port, 8000);
    // 形状仍是 P4 之前的（`backend` + 扁平字段）：补丁重写后同样生效
    assert_eq!(cfg.tts.engine, "xfyun");
    assert_eq!(cfg.tts.xfyun.voice, "xiaoyan");
    assert!(cfg.llm.stream);
    // 未提及的字段来自基线（默认配置）
    assert_eq!(cfg.tts.cache_entries, 256);
}

/// 持久化判定：目录挂载下的文件、文件级 bind mount 均算持久；容器层文件不算。
#[test]
fn persistence_detection() {
    let mi = concat!(
        "25 30 0:23 / /data rw,relatime - ext4 /dev/sda1 rw\n",
        "31 30 0:24 / /etc/hosts rw,relatime - ext4 /dev/sda1 rw\n",
        "40 30 0:26 / /etc/xiaozhi/config.toml rw,relatime - ext4 /dev/sda1 rw\n",
    );
    assert!(
        is_persistent_in(mi, "/data/config.toml"),
        "目录挂载下的文件应持久"
    );
    assert!(
        is_persistent_in(mi, "/etc/xiaozhi/config.toml"),
        "文件级 bind mount 应持久"
    );
    assert!(
        !is_persistent_in(mi, "/etc/xiaozhi/other.toml"),
        "容器层文件不应判持久"
    );
    assert!(
        !is_persistent_in(mi, "/datax/config.toml"),
        "前缀相似但非子路径不应误判"
    );
}

/// revision（内容指纹）：同内容稳定、内容变则变、**敏感字段也参与**
/// （否则"只改了密钥"会被误判为没变，过期写入检查就形同虚设）。
#[test]
fn revision_tracks_content() {
    let a = full_test_config();
    assert_eq!(
        config_revision(&a),
        config_revision(&a.clone()),
        "同内容必须稳定"
    );

    let mut b = a.clone();
    b.llm.model = "another-model".into();
    assert_ne!(
        config_revision(&a),
        config_revision(&b),
        "内容变化必须改变指纹"
    );

    // 密钥：**值**有意不参与指纹（见 `config_revision`，避免指纹成为密钥的比对依据），
    // 但**有无**（presence）必须参与 —— 否则"删除密钥"这类改动不会被其他端察觉。
    let mut c = a.clone();
    c.llm.api_key = "sk-changed".into();
    assert_eq!(
        config_revision(&a),
        config_revision(&c),
        "密钥值不参与指纹（有意为之）"
    );
    c.llm.api_key = String::new();
    assert_ne!(
        config_revision(&a),
        config_revision(&c),
        "密钥有无（presence）必须改变指纹"
    );

    // 逐字段：任何一处默认值/字段变动都应被察觉（这里抽查 audio 与 aiui）
    let mut d = a.clone();
    d.audio.downlink_lead_ms += 1;
    assert_ne!(config_revision(&a), config_revision(&d));
    let mut e = a.clone();
    e.aiui.enabled = !e.aiui.enabled;
    assert_ne!(config_revision(&a), config_revision(&e));
}

/// 客户端原样回传整份 GET 结果（含 revision 包装字段）时，包装字段不得进入配置结构。
#[test]
fn wrapper_keys_are_stripped_before_persist() {
    let mut v = serde_json::json!({
        "llm": {"model": "m1"},
        "revision": "deadbeef",
        "expected_revision": "deadbeef",
    });
    strip_wrapper_keys(&mut v);
    assert!(v.get("revision").is_none());
    assert!(v.get("expected_revision").is_none());
    assert_eq!(v["llm"]["model"], "m1", "业务字段不受影响");
    // 剔除后仍是合法 Config（未知键本就被忽略，双保险）
    let _: Config = serde_json::from_value(v).unwrap();
}

// ---------- 密钥打码 / presence / 空串语义 ----------

/// GET 打码：非空密钥被**摘除**只留 presence 标记，空密钥标记为 `false`，非密钥字段不受影响。
#[test]
fn persist_config_roundtrip() {
    let dir = std::env::temp_dir().join(format!("xz_cfg_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let cfg: Config = toml::from_str(
        r#"
[server]
port = 8000

[tts]
backend = "xfyun"

[tts.xfyun]
app_id = "a"
voice = "xiaoyan"
"#,
    )
    .unwrap();
    persist_config(path.to_str().unwrap(), &cfg).unwrap();
    let loaded = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(loaded.server.port, 8000);
    assert_eq!(loaded.tts.xfyun.voice, "xiaoyan");
    // `persist_config` 会先 normalize：旧 `backend` 落盘为 `engine`，值不丢
    assert_eq!(loaded.tts.engine, "xfyun");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 旧 schema 配置文件（含已废弃的 listen 键、P4 之前的 `[tts]` 扁平写法）应能静默解析
/// （未知键忽略），且规范化后 `backend`/扁平字段落到规范位置。
#[test]
fn legacy_config_with_listen_key_parses() {
    let legacy = r#"
[server]
listen = "192.168.99.250:8000"
expected_token = ""

[tts]
backend = "sherpa"
model = "/data/models/Kokoro/model.int8.onnx"
"#;
    let mut cfg: Config = toml::from_str(legacy).expect("旧 schema 配置应能解析");
    cfg.normalize();
    assert_eq!(cfg.server.port, 8000); // 缺省回退
    assert_eq!(cfg.tts.engine, "kokoro");
    assert_eq!(
        cfg.tts.kokoro.model,
        "/data/models/Kokoro/model.int8.onnx",
        "旧 [tts].model 必须搬到 [tts.kokoro]"
    );
}
