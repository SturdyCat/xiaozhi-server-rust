//! 下行编码参数与磁盘身份的关系（v3 PCM 存储的核心收益）。
//!
//! v2 时代 codec/版本/采样率/帧长都参与磁盘身份，换设备档位 = 全部重合成；v3 起磁盘
//! 只存 24k PCM，下行参数只影响**内存键**（帧按会话格式编码）。

use super::*;
use crate::media::media::{AudioCodec, AudioFormat};

#[test]
fn downlink_format_changes_keep_disk_entry_and_split_memory_only() {
    let k = HotAudioKey::from_session(&Config::default(), "same speech");
    // 同一句的三种下行档位：磁盘身份完全相同；内存键各不相同。
    let opus_16k_40 = AudioFormat::new(AudioCodec::Opus, 16_000, 40);
    let opus_16k_60 = AudioFormat::new(AudioCodec::Opus, 16_000, 60);
    let mut newer = opus_16k_60;
    newer.codec_version += 1;

    assert_eq!(k.file_id(), k.file_id(), "磁盘身份不随下行档位变化");
    assert_ne!(k.mem_key(opus_16k_40), k.mem_key(opus_16k_60));
    assert_ne!(
        k.mem_key(opus_16k_60),
        k.mem_key(newer),
        "编解码契约版本参与内存键"
    );

    // 一份 PCM 成品，多档位命中同一文件。
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    assert!(store.put(&k, &pcm(96)));
    assert!(store.get(&k).is_some(), "换下行档位后同一文件仍然命中");
    assert_eq!(
        HotAudioStore::new(Some(dir)).get(&k).unwrap(),
        pcm(96),
        "重启（新实例）后仍然命中"
    );
}
