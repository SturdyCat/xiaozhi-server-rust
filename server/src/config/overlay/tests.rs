#[cfg(test)]
mod overlay_tests {
    use crate::config::Config;
    use crate::config::overlay::{
        merge_overlay, overlay_allowed,
        validate_overlay,
    };

    #[test]
    fn whitelist_allows_session_internal_fields() {
        assert!(overlay_allowed(&["tts", "speaker"]));
        assert!(overlay_allowed(&["tts", "speed"]));
        assert!(overlay_allowed(&["tts", "xfyun", "voice"]));
        assert!(overlay_allowed(&["llm", "system_prompt"]));
        assert!(overlay_allowed(&["llm", "max_history"]));
        assert!(overlay_allowed(&["soul", "enabled"]));
        assert!(overlay_allowed(&["soul", "name"]));
        assert!(overlay_allowed(&["wakeup", "greeting"]));
        assert!(overlay_allowed(&["command", "keywords"]));
        assert!(overlay_allowed(&["memory", "recall_max_nodes"]));
        assert!(overlay_allowed(&["memory", "inject_position"]));
    }

    #[test]
    fn whitelist_rejects_structural_and_engine_fields() {
        assert!(!overlay_allowed(&["server", "port"]));
        assert!(!overlay_allowed(&["audio", "downlink_lead_ms"]));
        assert!(!overlay_allowed(&["aiui", "enabled"]));
        assert!(!overlay_allowed(&["tts", "engine"]));
        assert!(!overlay_allowed(&["tts", "kokoro", "lang"]));
        assert!(!overlay_allowed(&["tts", "kokoro", "model"]));
        assert!(!overlay_allowed(&["llm", "model"]));
        assert!(!overlay_allowed(&["memory", "engine"]));
        assert!(!overlay_allowed(&["memory", "enabled"]));
        assert!(!overlay_allowed(&["memory", "db_path"]));
        assert!(!overlay_allowed(&["memory", "extractor", "api_key"]));
    }

    #[test]
    fn validate_overlay_rejects_disallowed() {
        assert!(validate_overlay(&serde_json::json!({"server": {"port": 9000}})).is_err());
        assert!(validate_overlay(&serde_json::json!({"tts": {"kokoro": {"lang": "zh"}}})).is_err());
        assert!(validate_overlay(&serde_json::json!({"memory": {"engine": "store"}})).is_err());
    }

    #[test]
    fn validate_overlay_accepts_whitelisted() {
        let ok = serde_json::json!({
            "tts": {"speaker": 2, "speed": 1.2},
            "llm": {"system_prompt": "x", "max_history": 10},
            "soul": {"name": "z"},
            "command": {"keywords": "a"},
            "memory": {"recall_max_nodes": 9}
        });
        assert!(validate_overlay(&ok).is_ok());
    }

    #[test]
    fn merge_overlay_overrides_only_mentioned_whitelisted_fields() {
        let global = Config::default();
        let overlay = serde_json::json!({"tts": {"speaker": 7}, "llm": {"max_history": 99}});
        let merged = merge_overlay(&global, &overlay).unwrap();
        assert_eq!(merged.tts.speaker, 7);
        assert_eq!(merged.llm.max_history, 99);
        // 未提及字段保持全局值
        assert_eq!(merged.tts.speed, global.tts.speed);
        assert_eq!(merged.llm.system_prompt, global.llm.system_prompt);
    }

    #[test]
    fn merge_overlay_explicit_null_clears_to_default() {
        let mut global = Config::default();
        global.tts.speed = 2.0;
        let overlay = serde_json::json!({"tts": {"speed": null}});
        let merged = merge_overlay(&global, &overlay).unwrap();
        assert_eq!(merged.tts.speed, Config::default().tts.speed);
    }

    #[test]
    fn merge_overlay_preserves_unmentioned_sections() {
        let global = Config::default();
        let overlay = serde_json::json!({"soul": {"name": "foo"}});
        let merged = merge_overlay(&global, &overlay).unwrap();
        assert_eq!(merged.llm.model, global.llm.model);
        assert_eq!(merged.tts.engine, global.tts.engine);
        assert_eq!(merged.memory.engine, global.memory.engine);
    }

    #[test]
    fn merge_overlay_is_pure_and_session_internal_only() {
        // 核心不变量：overlay 是**纯函数**——绝不修改传入的全局 Config，只返回一份合并后的新 Config。
        // 引擎（TTS/LLM/Memory）只按**全局**配置构建（`Engines::live_config`），每设备生效配置
        // （`effective_config`）只在会话请求时现读 / 会话开始快照，永不喂给引擎构建，因此即便
        // 个别白名单字段（如 `llm.system_prompt`）恰好出现在某能力签名（`sig_llm`）里，也绝不会
        // 触发共享引擎重建——签名只从全局配置算、只用于全局引擎热切换判定。
        let global = Config::default();
        let speaker_before = global.tts.speaker;
        let greeting_before = global.wakeup.greeting.clone();
        let overlay = serde_json::json!({
            "tts": {"speaker": 3, "speed": 1.5},
            "llm": {"system_prompt": "p", "max_history": 12},
            "memory": {"recall_max_nodes": 11, "recall_max_tokens": 500},
            "soul": {"name": "s"},
            "command": {"keywords": ["k"]},
            "wakeup": {"greeting": "hi"}
        });
        let merged = merge_overlay(&global, &overlay).unwrap();
        // 1) 全局 Config 未被修改（纯函数：`merge_overlay(&Config)` 只接受不可变引用，绝不回写全局）
        assert_eq!(global.tts.speaker, speaker_before, "merge_overlay 不应修改传入的全局配置");
        assert_eq!(global.wakeup.greeting, greeting_before, "merge_overlay 不应修改传入的全局配置");
        // 2) overlay 确实生效（只覆盖提到的白名单字段）
        assert_eq!(merged.tts.speaker, 3);
        assert_eq!(merged.llm.system_prompt, "p");
        assert_eq!(merged.llm.max_history, 12);
        assert_eq!(merged.memory.recall_max_nodes, 11);
        assert_eq!(merged.memory.recall_max_tokens, 500);
        assert_eq!(merged.soul.name, "s");
        assert_eq!(merged.wakeup.greeting, "hi");
        // 3) 未提及字段保持全局值（overlay 只覆盖它实际含有的字段）
        assert_eq!(merged.tts.engine, global.tts.engine);
        assert_eq!(merged.llm.model, global.llm.model);
        assert_eq!(merged.memory.engine, global.memory.engine);
    }
}
