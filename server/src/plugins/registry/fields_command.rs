//! end_session训练词表元数据；旧规则字段只反序列化兼容，不再向管理表单暴露。

use super::{FieldKind, FieldSchema, HotReload};

pub(super) const COMMAND_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        key: "enabled", label: "启用结束聆听技能", kind: FieldKind::Bool, default_hint: "true",
        help: "将保存态词表编入end_session统一训练目录；运行必须由真实模型接受，不做关键词命中。模型未启用时不执行。关闭仅移除此技能，不影响普通聊天。",
        hot: HotReload::NextSession, required: true,
    },
    FieldSchema {
        key: "keywords", label: "结束聆听训练样例", kind: FieldKind::Text, default_hint: "退下 / 闭嘴 / 关闭",
        help: "每行一个完整训练句，与保存态训练输入/manifest/候选目录同源。修改后须重新训练验证，运行期不按exact或contains匹配；训练样本不足应补充真实变体，不复制同句训练与测试。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "reply", label: "告别语", kind: FieldKind::Str, default_hint: "好的，我先退下了。",
        help: "本轮模型接受end_session后先说告别语，再由会话授权让设备结束聆听；留空只执行结束动作。设备MCP与老固件兼容策略保留，不能绕过模型门禁。",
        hot: HotReload::NextSession, required: false,
    },
];
