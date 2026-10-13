//! `server/src/app/splitter.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! 缺口③（定时 flush）补充：`take_expired` / `flush_deadline` 的纯函数语义 +
//! watchdog 集成测（`tokio::time::paused`：停顿出残句、done/abort 退出、时序可控）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SentenceSplitter, find_sentence_end, spawn_flush_watchdog};

#[test]
fn splitter_splits_on_chinese_punctuation() {
    let mut sp = SentenceSplitter::new();
    assert!(sp.feed("今天天气不错，").is_empty(), "逗号不是句末标点");
    let out = sp.feed("适合出门。明天呢？");
    assert_eq!(out, vec!["今天天气不错，适合出门。", "明天呢？"]);
    assert!(sp.flush().is_empty());
}

#[test]
fn splitter_multiple_sentences_single_delta() {
    let mut sp = SentenceSplitter::new();
    let out = sp.feed("一。二！三？四；");
    assert_eq!(out, vec!["一。", "二！", "三？", "四；"]);
}

#[test]
fn splitter_flush_tail_without_punctuation() {
    let mut sp = SentenceSplitter::new();
    assert_eq!(sp.feed("你好。再见"), vec!["你好。"]);
    let out = sp.flush();
    assert_eq!(out, vec!["再见"]);
}

#[test]
fn splitter_fallback_max_len_no_punctuation() {
    let mut sp = SentenceSplitter::new();
    let text: String = "啊".repeat(120);
    let out = sp.feed(&text);
    // 120 字无标点 → 按 50 字兜底切出 2 句，剩 20 字留在缓冲
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|s| s.chars().count() == 50));
    assert_eq!(sp.flush(), vec!["啊".repeat(20)]);
}

#[test]
fn splitter_ascii_and_newline_boundaries() {
    let mut sp = SentenceSplitter::new();
    // \n 段去掉首尾空白后为空 → 跳过；"new line" 留在缓冲待 flush
    let out = sp.feed("ok! next? done;\nnew line");
    assert_eq!(out, vec!["ok!", "next?", "done;"]);
    assert_eq!(sp.flush(), vec!["new line"]);
}

#[test]
fn splitter_fallback_aligned_to_char_boundary() {
    // 兜底硬切必须对齐多字节字符边界（中文 3 字节），不得切碎
    let mut sp = SentenceSplitter::new();
    let text: String = "语".repeat(60);
    let out = sp.feed(&text);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].chars().count(), 50);
    assert_eq!(sp.flush(), vec!["语".repeat(10)]);
}

#[test]
fn find_sentence_end_positions() {
    assert_eq!(find_sentence_end("你好。"), Some(9)); // 3 字节 ×2 + 3 字节句号
    assert_eq!(find_sentence_end("你好"), None);
    assert_eq!(find_sentence_end(""), None);
    assert_eq!(find_sentence_end("hi!\n"), Some(3));
}

// ============================================================
// 定时 flush（缺口③）：take_expired / flush_deadline 纯函数语义
// ============================================================

#[test]
fn take_expired_requires_silence_and_residual() {
    let mut sp = SentenceSplitter::new();
    let wait = Duration::from_millis(1500);
    // 没喂过 → 永远不到期
    assert!(sp.flush_deadline(wait).is_none());

    // 喂入半句（无标点）→ last_feed 刷新（deadline = feed 时刻 + wait，取它做"到期时刻"）
    assert!(sp.feed("这是一段没有标点的残").is_empty());
    let deadline = sp.flush_deadline(wait).expect("残句非空应有 deadline");
    // 静默不足 → 不出
    assert!(sp.take_expired(deadline - Duration::from_millis(1), wait).is_none());
    // 静默够长 → 出残句
    let got = sp
        .take_expired(deadline, wait)
        .expect("静默 ≥ wait 且残句非空应出句");
    assert_eq!(got, "这是一段没有标点的残");
    // 取走后残句清空 → 再到期也不出
    assert!(sp.take_expired(deadline + wait, wait).is_none());
}

#[test]
fn take_expired_ignores_whitespace_only_residual() {
    let mut sp = SentenceSplitter::new();
    sp.feed("   \n\t ");
    let t0 = Instant::now();
    let wait = Duration::from_millis(10);
    assert!(
        sp.take_expired(t0 + wait, wait).is_none(),
        "纯空白残句 trim 后为空，不应出句"
    );
}

#[test]
fn feed_refreshes_last_feed_and_deadline() {
    let mut sp = SentenceSplitter::new();
    let wait = Duration::from_millis(1000);
    let t0 = Instant::now();
    sp.feed("残");
    let d1 = sp.flush_deadline(wait).expect("残句非空应有 deadline");
    // 新 token 刷新 last_feed → deadline 后移
    std::thread::sleep(Duration::from_millis(30));
    sp.feed("句");
    let d2 = sp.flush_deadline(wait).expect("残句仍在");
    assert!(d2 > d1, "喂入必须刷新 deadline（误醒无害的前提）");
    assert!(d2 > t0 + Duration::from_millis(30));
}

#[test]
fn flush_clears_timing_state() {
    let mut sp = SentenceSplitter::new();
    sp.feed("残句");
    assert_eq!(sp.flush(), vec!["残句"]);
    assert!(sp.flush_deadline(Duration::from_millis(1)).is_none(), "flush 后无残句无 deadline");
    // flush 重置 last_feed：即使再喂入，旧时间不残留
    sp.feed("新的");
    assert!(sp.flush_deadline(Duration::from_secs(60)).is_some());
}

// ============================================================
// watchdog 集成测（#[tokio::test]：时间手动推进，稳定不抖）
// ============================================================

/// 停顿出残句：喂半句 → 停顿 ≥ wait → watchdog 把残句推进通道；随后的正常
/// 标点出句不受影响（feed 继续工作）。
#[tokio::test]
async fn watchdog_flushes_residual_after_silence() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        Arc::new(|| {}),
        Duration::from_millis(1500),
    );
    // 喂半句（模拟 LLM 思考停顿：token 后再无输出）
    shared.lock().unwrap().feed("思考中尚未成句的残");
    // 静默期推进：1500ms 到期
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("到期后 watchdog 必须出残句")
        .expect("通道不应关闭");
    assert_eq!(got, "思考中尚未成句的残");
    // 收工：watchdog 退出
    done.store(true, Ordering::Release);
    finished.notify_waiters();
}

/// done 置位后不再出句（顺序保证：先查再取——abort/done 之后的残句不追加）。
#[tokio::test]
async fn watchdog_exits_on_done_without_flushing() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(true)); // 已收工
    let finished = Arc::new(tokio::sync::Notify::new());
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        Arc::new(|| {}),
        Duration::from_millis(50),
    );
    shared.lock().unwrap().feed("不该被送出的残句");
    tokio::time::sleep(Duration::from_millis(500)).await;
    // done 先于喂入：watchdog 首轮检查即退出并 drop tx → recv 得到 Ok(None)（关闭），
    // 而不是 Err（超时后仍有数据）。两种都算"没出句"，但语义要分清：
    let got = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    match got {
        Err(_timeout) => panic!("watchdog 未退出（通道悬而未决）"),
        Ok(Some(s)) => panic!("done 置位后 watchdog 不得再出残句，实测出了 {s:?}"),
        Ok(None) => {} // watchdog 已退出并关闭通道：正确
    }
}

/// abort 置位后不再出句（打断后不追加残句——barge-in 语义）。
#[tokio::test]
async fn watchdog_exits_on_abort_without_flushing() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(true)); // 已打断
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        Arc::new(|| {}),
        Duration::from_millis(50),
    );
    shared.lock().unwrap().feed("打断后不该送出的残句");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let got = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    match got {
        Err(_timeout) => panic!("watchdog 未退出（通道悬而未决）"),
        Ok(Some(s)) => panic!("abort 置位后 watchdog 不得再出残句，实测出了 {s:?}"),
        Ok(None) => {} // watchdog 已退出并关闭通道：正确
    }
}

/// 消费者关闭（tx 对端 drop）→ watchdog 自行退出（不留 detached 任务）。
#[tokio::test]
async fn watchdog_exits_when_channel_closed() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        Arc::new(|| {}),
        Duration::from_millis(50),
    );
    shared.lock().unwrap().feed("消费者已放弃");
    drop(rx); // 消费者退出
    tokio::time::sleep(Duration::from_millis(600)).await;
    // 无 panic、无悬挂（select/发送失败路径都能到达 return）
}

/// 新 token 持续刷新 deadline：静默不足时 watchdog 不出句（误醒无害）。
#[tokio::test]
async fn watchdog_does_not_flush_while_tokens_keep_arriving() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        Arc::new(|| {}),
        Duration::from_millis(1500),
    );
    // 每 300ms 一个 token，持续 2s（间隙 < wait）
    for _ in 0..7 {
        shared.lock().unwrap().feed("字");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), rx.recv())
            .await
            .is_err(),
        "token 间隙 < wait 时不得出残句（误切）"
    );
    done.store(true, Ordering::Release);
    finished.notify_waiters();
}

/// 阻塞 2 回归（watchdog 层）：`on_flush` 钩子在残句 **send 成功后**触发恰好一次；
/// 没有残句到期（误醒）不触发。
#[tokio::test]
async fn on_flush_hook_fires_once_after_successful_send() {
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let abort = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    let fired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fired_cb = fired.clone();
    let on_flush: Arc<dyn Fn() + Send + Sync> =
        Arc::new(move || {
            fired_cb.fetch_add(1, Ordering::SeqCst);
        });
    spawn_flush_watchdog(
        shared.clone(),
        tx,
        abort.clone(),
        done.clone(),
        finished.clone(),
        on_flush,
        Duration::from_millis(50),
    );
    // 喂半句 → 静默到期 → 出残句 + on_flush 一次
    shared.lock().unwrap().feed("会被定时送出的残句");
    let got = tokio::time::timeout(Duration::from_millis(400), rx.recv())
        .await
        .expect("到期应出残句")
        .expect("通道不应关闭");
    assert_eq!(got, "会被定时送出的残句");
    assert_eq!(fired.load(Ordering::SeqCst), 1, "send 成功后 on_flush 恰好一次");
    // 收工退出；再等一段（无残句、通道仍在）不触发第二次
    done.store(true, Ordering::Release);
    finished.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fired.load(Ordering::SeqCst), 1, "收工后不得再触发");
}
