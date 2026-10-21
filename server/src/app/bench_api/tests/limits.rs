use super::*;
use crate::plugins::{
    asr::AsrEngine,
    llm::{LlmEvent, LlmProvider, LlmTurnResult, ToolSpec},
    registry::{Built, Capability},
    skill::SkillRegistry,
    tts::{TtsChunkCallback, TtsEngine},
    vad::VadEngine,
};
use std::{future::Future, pin::Pin};

struct ExcessOutput;
impl providers::ProviderFactory for ExcessOutput {
    fn build(&self, _: &Config, cap: Capability) -> anyhow::Result<Built> {
        match cap {
            Capability::Asr => Ok(Built::Asr(Arc::new(Self))),
            Capability::Llm => Ok(Built::Llm(Arc::new(Self))),
            Capability::Tts => Ok(Built::Tts(Arc::new(Self))),
            _ => anyhow::bail!("unsupported"),
        }
    }
    fn vad(&self, cfg: &Config) -> anyhow::Result<Box<dyn VadEngine>> {
        FakeProviders::default().vad(cfg)
    }
    fn skills(&self, cfg: &Config) -> anyhow::Result<SkillRegistry> {
        FakeProviders::default().skills(cfg)
    }
}
impl AsrEngine for ExcessOutput {
    fn recognize(&self, _: &[f32], _: u32) -> anyhow::Result<String> {
        Ok("x".repeat(16385))
    }
}
impl LlmProvider for ExcessOutput {
    fn chat_stream<'a>(
        &'a self,
        _: &'a [(String, String)],
        _: &'a str,
        _: &'a TurnPrompt,
        _: Option<&'a [ToolSpec]>,
        mut cb: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            cb(LlmEvent::Text("x".repeat(16385)));
            Ok(LlmTurnResult {
                text: "partial".into(),
                ..Default::default()
            })
        })
    }
}
impl TtsEngine for ExcessOutput {
    fn synthesize_stream(
        &self,
        _: &str,
        _: f32,
        _: i32,
        mut cb: TtsChunkCallback,
    ) -> anyhow::Result<()> {
        let chunk = vec![0.1; 24000];
        for _ in 0..61 {
            if !cb(24000, &chunk) {
                break;
            }
        }
        Ok(())
    }
}
#[tokio::test]
async fn every_provider_output_is_bounded_on_the_actual_service_path() {
    let (service, store) = service(&Config::default(), Arc::new(ExcessOutput));
    for action in ["asr.sample", "llm.chat", "tts.synthesis"] {
        let out = service.run(request(action, Some("hello"))).await;
        assert_eq!(out.result.state, "error", "{action}");
        assert_eq!(out.result.error_code, Some("output_limit"), "{action}");
        assert!(out.result.text.is_none());
        assert_ne!(out.result.readiness, "provider_verified");
    }
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}
#[tokio::test]
async fn pcm_over_duration_limit_is_rejected_before_inference() {
    use base64::Engine;
    let factory = Arc::new(FakeProviders::default());
    let (service, _) = service(&Config::default(), factory.clone());
    let audio = json!({"format":"pcm_s16le","sample_rate":16000,"channels":1,"data_base64":base64::engine::general_purpose::STANDARD.encode(vec![0;480002])});
    let req = serde_json::from_value(json!({"action_id":"asr.recording","audio":audio})).unwrap();
    assert_eq!(
        service.run(req).await.result.error_code,
        Some("invalid_input")
    );
    assert!(factory.seen.lock().unwrap().is_empty());
}
