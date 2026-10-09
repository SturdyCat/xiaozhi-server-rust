use super::*;

/// 注册表 id 必须命名空间唯一（`<capability>.<impl>`）。
#[test]
fn descriptor_ids_are_unique_and_namespaced() {
    let mut seen = std::collections::HashSet::new();
    for d in REGISTRY {
        assert!(seen.insert(d.id), "重复的插件 id: {}", d.id);
        let (cap, impl_name) =
            d.id.split_once('.')
                .unwrap_or_else(|| panic!("id 必须是 <capability>.<impl> 形式: {}", d.id));
        assert_eq!(
            cap,
            d.capability.id(),
            "id 前缀与 capability 不一致: {}",
            d.id
        );
        assert!(!impl_name.is_empty(), "缺少实现名: {}", d.id);
    }
}

/// 每个**已实现**的能力至少有一个实现（防止"声明了能力却没人实现"）。
#[test]
fn every_implemented_capability_has_an_impl() {
    for &cap in ALL_CAPABILITIES {
        if !cap.implemented() {
            continue;
        }
        assert!(
            impls_of(cap).count() > 0,
            "能力 {} 已声明实现但注册表里没有条目",
            cap.id()
        );
    }
}

/// 必需能力必须有**可选中的**实现（否则启动必然失败且无从下手）。
#[test]
fn required_capabilities_have_selectable_impl() {
    let cfg = Config::default();
    for &cap in ALL_CAPABILITIES {
        if !cap.required() {
            continue;
        }
        assert!(
            active_impl(&cfg, cap).is_some(),
            "必需能力 {} 在默认配置下没有任何被选中的实现",
            cap.id()
        );
    }
}

/// TTS/FullChain 的选中判定随配置切换（多实现互斥）。
#[test]
fn selection_follows_config() {
    let mut cfg = Config::default();
    assert_eq!(active_impl(&cfg, Capability::Tts), Some("tts.kokoro"));
    cfg.tts.engine = "xfyun".into();
    assert_eq!(active_impl(&cfg, Capability::Tts), Some("tts.xfyun"));
    assert_eq!(active_impl(&cfg, Capability::FullChain), None);
    cfg.aiui.enabled = true;
    assert_eq!(
        active_impl(&cfg, Capability::FullChain),
        Some("fullchain.aiui")
    );
    // 记忆/人格：默认关闭 → 没有选中实现；打开后各有一个
    assert_eq!(active_impl(&cfg, Capability::Memory), None);
    assert_eq!(active_impl(&cfg, Capability::Soul), None);
    cfg.memory.enabled = true;
    cfg.soul.enabled = true;
    assert_eq!(active_impl(&cfg, Capability::Memory), Some("memory.graph"));
    assert_eq!(active_impl(&cfg, Capability::Soul), Some("soul.profile"));
    // backend = none 时不选中（保留配置但不生效）
    cfg.memory.engine = crate::plugins::memory::MemoryBackend::None;
    assert_eq!(active_impl(&cfg, Capability::Memory), None);
}

/// 记忆/人格必须上报 `next_session`（保存后新会话生效），且都能通过校验。
#[test]
fn context_capabilities_validate_and_report_hot() {
    let mut cfg = Config::default();
    cfg.memory.enabled = true;
    cfg.soul.enabled = true;
    cfg.soul.max_sentences = 1;
    cfg.soul.self_intro = "我是 {{name}}".into();
    for d in REGISTRY {
        if !matches!(d.capability, Capability::Memory | Capability::Soul) || !d.is_selected(&cfg) {
            continue;
        }
        (d.validate)(&cfg).unwrap_or_else(|e| panic!("{} 校验应通过: {e:#}", d.id));
        assert_eq!(d.hot, HotReload::NextSession, "{} 应声明为 next_session", d.id);
        assert!(
            d.requires.is_empty() || d.capability == Capability::Memory,
            "{} 不应把上游依赖声明为硬前置（坏掉只降级）",
            d.id
        );
    }
    // 记忆的软性问题（无 embedding）必须能被上报为降级原因
    let warn = REGISTRY
        .iter()
        .find(|d| d.id == "memory.graph")
        .and_then(|d| d.warn_if)
        .and_then(|f| f(&cfg))
        .expect("无 embedding 时应给出降级原因");
    assert!(warn.contains("词法"), "{warn}");
}

/// 默认配置下必需能力可通过前置校验（非 sherpa 构建除外——那是编译期事实）。
#[test]
fn default_config_validates_when_feature_enabled() {
    let cfg = Config::default();
    for d in REGISTRY {
        if !d.capability.required() || !d.is_selected(&cfg) {
            continue;
        }
        if !cfg!(feature = "sherpa")
            && (d.capability == Capability::Asr || d.capability == Capability::Tts)
        {
            // 无 feature 时必须给出「怎么修」的明确报错，而不是底层 ctor 报错
            let err = (d.validate)(&cfg).unwrap_err().to_string();
            assert!(
                err.contains("sherpa"),
                "报错应说明需要 sherpa feature: {err}"
            );
            continue;
        }
        (d.validate)(&cfg).unwrap_or_else(|e| panic!("默认配置下 {} 校验应通过: {e:#}", d.id));
    }
}

/// **漂移护栏**：schema 里声明的每个字段 key 必须在 `Config` 里真实存在。
/// 新增/重命名字段时若忘了改注册表，本测试直接打红。
#[test]
fn field_keys_exist_in_config() {
    let cfg_json = serde_json::to_value(Config::default()).expect("Config → JSON");
    let lookup = |path: &[&str], key: &str| -> bool {
        let mut node = &cfg_json;
        for seg in path {
            match node.get(*seg) {
                Some(n) => node = n,
                None => return false,
            }
        }
        node.get(key).is_some()
    };

    assert!(
        !CAPABILITY_COMMON_FIELDS.is_empty() && !CONTEXT_COMMON_FIELDS.is_empty(),
        "公共字段表不应为空（tts backend/cache_entries + soul/memory）"
    );
    for &cap in ALL_CAPABILITIES {
        for (path, fields) in capability_common_fields(cap) {
            for f in fields {
                assert!(
                    lookup(path, f.key),
                    "能力 {} 的公共字段 `{}` 在 Config 路径 {:?} 中不存在（字段被改名或删除？）",
                    cap.id(),
                    f.key,
                    path
                );
            }
        }
    }
    for d in REGISTRY {
        if d.fields.is_empty() {
            continue;
        }
        assert!(
            !d.fields_path.is_empty(),
            "{} 声明了字段但没有 fields_path",
            d.id
        );
        for f in d.fields {
            assert!(
                lookup(d.fields_path, f.key),
                "{} 的字段 `{}` 在 Config 路径 {:?} 中不存在（字段被改名或删除？）",
                d.id,
                f.key,
                d.fields_path
            );
        }
    }
}

/// 同一路径下的字段 key 不得重复（重复会让 UI 渲染两次、语义歧义）。
#[test]
fn field_keys_are_unique_per_path() {
    for d in REGISTRY {
        let mut seen = std::collections::HashSet::new();
        for f in d.fields {
            assert!(seen.insert(f.key), "{} 重复声明字段 {}", d.id, f.key);
        }
    }
    for &cap in ALL_CAPABILITIES {
        for (path, fields) in capability_common_fields(cap) {
            let mut seen = std::collections::HashSet::new();
            for f in fields {
                assert!(
                    seen.insert(f.key),
                    "能力 {} 的公共字段 {} 在 {:?} 重复",
                    cap.id(),
                    f.key,
                    path
                );
            }
        }
    }
}

/// 密钥字段必须用 `Password` 类型（否则管理页会回显明文）。
#[test]
fn credential_fields_are_marked_secret() {
    let secret_keys = ["api_key", "api_secret"];
    let check = |owner: &str, f: &FieldSchema| {
        if secret_keys.contains(&f.key) {
            assert!(
                f.kind.is_secret(),
                "{owner} 的 {} 应标记为 Password（密钥不可回显）",
                f.key
            );
        }
    };
    for d in REGISTRY {
        for f in d.fields {
            check(d.id, f);
        }
    }
    for &cap in ALL_CAPABILITIES {
        for (_, fields) in capability_common_fields(cap) {
            for f in fields {
                check(cap.id(), f);
            }
        }
    }
}

/// schema JSON 的形状（前端契约）：能力数 = 全部能力，字段带 secret/hot 标记。
#[test]
fn schema_json_shape() {
    let v = schema_json();
    let caps = v["capabilities"].as_array().expect("capabilities 数组");
    assert_eq!(caps.len(), ALL_CAPABILITIES.len());
    let tts = caps
        .iter()
        .find(|c| c["id"] == "tts")
        .expect("应有 tts 能力");
    assert_eq!(tts["required"], true);
    assert_eq!(tts["implementations"].as_array().unwrap().len(), 2);
    let xfyun = tts["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == "tts.xfyun")
        .unwrap();
    let api_key = xfyun["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "api_key")
        .unwrap();
    assert_eq!(api_key["secret"], true, "密钥字段必须带 secret 标记");
    assert_eq!(api_key["hot"], "next_session");
    assert_eq!(api_key["kind"], "password");
    // `app_sections`：非能力段（`[server]` / `[audio]`）的档位投影——管理页据此显示
    // "这张卡片多久生效"（此前只能靠卡片文案口口相传，改了注册表也不会同步）
    let app = v["app_sections"].as_array().expect("app_sections 数组");
    let ids: Vec<&str> = app.iter().filter_map(|s| s["id"].as_str()).collect();
    assert!(ids.contains(&"server") && ids.contains(&"audio"), "{ids:?}");
    for s in app {
        assert!(s["hot_hint"].as_str().is_some_and(|h| !h.is_empty()), "{s}");
        assert_ne!(s["hot"], "live", "应用级段不可能是「保存即生效」: {s}");
    }
}

/// 指令闸门（`[command]`）的注册表语义护栏。
///
/// 它有两个"必须"：① 默认不选中（加插件不许改变现有部署行为）；
/// ② 启用后必须**新会话生效**且**字段档位有出处**（否则管理页永远显示"需重启"，属静默不正确）。
/// 坏配置（启用但词表为空）只能降级、不能拖垮语音服务，因此也不能是必需能力。
#[test]
fn command_gate_is_optional_and_hot_reloadable() {
    let mut cfg = Config::default();
    assert!(!cfg.command.enabled, "指令闸门默认必须关闭");
    assert_eq!(active_impl(&cfg, Capability::Command), None);
    assert!(
        !Capability::Command.required() && !Capability::Command.fatal_if_enabled(),
        "可选能力：坏配置只降级，不阻止启动"
    );

    cfg.command.enabled = true;
    assert_eq!(active_impl(&cfg, Capability::Command), Some("command.gate"));
    let d = find("command.gate").expect("应注册 command.gate");
    (d.validate)(&cfg).unwrap_or_else(|e| panic!("默认词表应通过校验: {e:#}"));
    assert_eq!(d.hot, HotReload::NextSession);
    // 字段级档位：改 enabled/keywords 都是"新会话生效"（`POST /api/config` 的 hot 据此判定）
    assert_eq!(
        field_hot(&["command", "enabled"]),
        Some(HotReload::NextSession)
    );
    assert_eq!(
        field_hot(&["command", "keywords"]),
        Some(HotReload::NextSession)
    );
    // 长度闸门同样必须是"新会话生效"（漏了它管理页会永远显示"需重启"）
    assert_eq!(
        field_hot(&["command", "max_chars"]),
        Some(HotReload::NextSession)
    );
    assert_eq!(section_hot("command"), HotReload::NextSession);

    // 启用但词表为空 = 闸门不生效：必须能被说成"配置问题"（而不是静默不拦）
    cfg.command.keywords.clear();
    let err = (d.validate)(&cfg).unwrap_err().to_string();
    assert!(err.contains("keywords"), "{err}");

    // schema 必须把这张卡的字段暴露出去（管理页「指令」页签的数据源）
    let v = schema_json();
    let cmd = v["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "command")
        .expect("schema 应包含 command 能力");
    let keys: Vec<&str> = cmd["implementations"][0]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["key"].as_str())
        .collect();
    assert_eq!(
        keys,
        vec!["enabled", "keywords", "match_mode", "max_chars", "reply"],
        "{keys:?}"
    );
}
