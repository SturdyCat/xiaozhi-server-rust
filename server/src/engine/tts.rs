//! 原子 TTS 租约：实例、实际构建配置、签名和代际作为同一快照发布。
//! 租约不实现 Debug：签名/配置可能含凭据，不能进入日志。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::config::Config;
use crate::plugins::host::Phase;
use crate::plugins::registry::{self, Capability};
use crate::plugins::tts::{build_kokoro_with_lang, TtsConfig, TtsEngine};

use super::{capability_signature, Engines};

/// 一次实际构建的共享所有权；持有它的旧会话不会被后续热刷新切换。
#[derive(Clone)]
pub struct TtsLease {
    engine: Arc<dyn TtsEngine>,
    config: Arc<TtsConfig>,
    fingerprint: Arc<str>,
    generation: u64,
}

impl TtsLease {
    pub fn engine(&self) -> Arc<dyn TtsEngine> {
        self.engine.clone()
    }
    pub fn config(&self) -> &TtsConfig {
        &self.config
    }
    /// 仅供缓存身份比较；可能含密钥，禁止打印。
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// overlay 只允许调用参数，不得用请求配置冒充实际模型/凭据。
    pub fn with_synthesis_params(&self, speaker: i32, speed: f32) -> Self {
        let mut config = (*self.config).clone();
        config.speaker = speaker;
        config.speed = speed;
        Self {
            config: Arc::new(config),
            ..self.clone()
        }
    }
}

type LanguageCell = Arc<OnceLock<Result<TtsLease, String>>>;

pub(super) struct TtsSlot {
    current: RwLock<TtsLease>,
    /// 构建串行化：后发刷新不能被先发慢构建覆盖，失败绝不拆旧租约。
    refresh: Mutex<()>,
    pool: Mutex<HashMap<(u64, String), LanguageCell>>,
}

impl TtsSlot {
    pub(super) fn new(cfg: &Config, engine: Arc<dyn TtsEngine>) -> Self {
        Self {
            current: RwLock::new(TtsLease {
                engine,
                config: Arc::new(cfg.tts.clone()),
                fingerprint: capability_signature(cfg, Capability::Tts).into(),
                generation: 1,
            }),
            refresh: Mutex::new(()),
            pool: Mutex::new(HashMap::new()),
        }
    }
    pub(super) fn lease(&self) -> TtsLease {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(super) fn refresh_with(
        &self,
        cfg: &Config,
        build: impl FnOnce(&Config) -> Result<Arc<dyn TtsEngine>, String>,
    ) -> Result<bool, String> {
        let _refresh = self.refresh.lock().unwrap_or_else(|e| e.into_inner());
        let old = self.lease();
        let fingerprint = capability_signature(cfg, Capability::Tts);
        let changed = fingerprint != old.fingerprint();
        let engine = if changed { build(cfg)? } else { old.engine() };
        let lease = TtsLease {
            engine,
            config: Arc::new(cfg.tts.clone()),
            fingerprint: fingerprint.into(),
            generation: old.generation + u64::from(changed),
        };
        // 发布与池失效同一事务；新代读者不能在发布/clear 间隙预留随后被误清的 cell。
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        if changed {
            self.pool.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        *current = lease;
        Ok(changed)
    }

    fn language_lease_with(
        &self,
        lang: &str,
        build: impl FnOnce(&TtsConfig, &str) -> Result<Arc<dyn TtsEngine>, String>,
    ) -> TtsLease {
        let lang = lang.trim();
        // 短锁只预留 cell；不同语言可以独立构建，锁序始终 current → pool。
        let (base, key, cell) = {
            let current = self.current.read().unwrap_or_else(|e| e.into_inner());
            let base = current.clone();
            if lang.is_empty()
                || base.config.engine_id() != "kokoro"
                || lang == base.config.kokoro.lang
            {
                return base;
            }
            let key = (base.generation(), lang.to_string());
            let cell = self
                .pool
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.clone())
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone();
            (base, key, cell)
        };
        let result = cell.get_or_init(|| {
            let engine = build(&base.config, lang)?;
            let mut cfg = Config::default();
            cfg.tts = (*base.config).clone();
            cfg.tts.kokoro.lang = lang.to_string();
            Ok(TtsLease {
                engine,
                config: Arc::new(cfg.tts.clone()),
                fingerprint: capability_signature(&cfg, Capability::Tts).into(),
                generation: base.generation(),
            })
        });
        match result {
            Ok(lease) => lease.with_synthesis_params(base.config.speaker, base.config.speed),
            Err(error) => {
                // 本 flight 的等待者共享失败；移除失败 cell 允许后续请求重试。
                // 换代会清旧 cell；这里绝不把过期构建重新插回当前池。
                let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
                if pool
                    .get(&key)
                    .is_some_and(|active| Arc::ptr_eq(active, &cell))
                {
                    pool.remove(&key);
                }
                tracing::warn!("构建 lang={lang} 的 TTS 失败，保留默认实际租约: {error}");
                base
            }
        }
    }
}

impl Engines {
    pub fn tts_lease(&self) -> TtsLease {
        self.tts.lease()
    }
    /// 兼容不需要缓存身份的测试台调用；生产会话必须固定持有 tts_lease。
    #[allow(dead_code)]
    pub fn tts(&self) -> Arc<dyn TtsEngine> {
        self.tts_lease().engine()
    }
    pub fn tts_name(&self) -> &'static str {
        self.tts_lease().engine.name()
    }
    /// 按实际构建代际隔离语言池；构建失败返回默认实际租约，不伪造请求语言身份。
    pub fn tts_lease_for(&self, lang: &str) -> TtsLease {
        if !cfg!(feature = "sherpa") {
            return self.tts_lease();
        }
        self.tts.language_lease_with(lang, |cfg, lang| {
            build_kokoro_with_lang(&cfg.kokoro, lang).map_err(|e| format!("{e:#}"))
        })
    }
    pub fn tts_for(&self, lang: &str) -> Arc<dyn TtsEngine> {
        self.tts_lease_for(lang).engine()
    }
    pub(super) fn refresh_tts(&self, fresh: &Config) -> Result<(), String> {
        let old = RwLock::new(self.tts_lease().fingerprint().to_string());
        if let Err(msg) = super::decide_refresh(Capability::Tts, &old, fresh) {
            self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
            return Err(msg);
        }
        let outcome = self.tts.refresh_with(fresh, |cfg| {
            let descriptor = registry::active_impl(cfg, Capability::Tts)
                .and_then(registry::find)
                .ok_or_else(|| "TTS 无匹配实现（保持当前引擎）".to_string())?;
            let build = descriptor
                .build
                .ok_or_else(|| "TTS 缺少构建入口".to_string())?;
            match build(cfg) {
                Ok(registry::Built::Tts(engine)) => Ok(engine),
                Ok(_) => Err("TTS 构建结果类型错误（保持当前引擎）".into()),
                Err(e) => Err(format!("TTS 引擎切换失败（保持当前引擎）: {e:#}")),
            }
        });
        match outcome {
            Ok(changed) => {
                // 缓存键含实际签名；旧会话继续复用其旧键，不必清掉仍有效的音频。
                self.inference.reconfigure_tts(fresh.tts.max_concurrent);
                self.mark_capability(Capability::Tts, Phase::Active, None);
                if changed {
                    tracing::info!("TTS 引擎已热切换（新会话生效）");
                }
                Ok(())
            }
            Err(msg) => {
                self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
                Err(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests;
