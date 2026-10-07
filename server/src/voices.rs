//! 讯飞 TTS 发音人目录（服务端为唯一数据源）：客户端（macApp 测试台/管理页）
//! 经 `GET /api/tts/voices` 动态获取，不再硬编码音色表——新增/过滤音色只改这里。
//!
//! 目录内容 = **实测可调用**的音色（2026-10-07 逐个冒烟验证）+ 控制台元数据
//! （官方名/场景/音色类型）。分组按用户规范：`male`（男）/ `female`（女）/
//! `x6`（极速拟人，x5/x6 全系跨性别）/ 其余由客户端走「自定义」手填 vcn。
//! x4_* 超拟人在经典 v2/tts 接口未授权（实测 11200），不收录。

use axum::{response::{IntoResponse, Response}, routing::get, Json, Router};

/// 目录条目。
#[derive(Debug, Clone, serde::Serialize)]
struct Voice {
    vcn: &'static str,
    name: &'static str,
    /// 性别主分组：female（女）/ male（男）。
    gender: &'static str,
    /// 音色类型：classic（普通发音人）/ x6（极速拟人，x5/x6 全系）。
    #[serde(rename = "type")]
    type_: &'static str,
    /// 附加描述（场景 · 音色 等；空串则只显示名字）。
    tag: &'static str,
}

/// 实测目录（73 个：女经典 10 + 男经典 4 + 极速拟人 59）。
const VOICES: &[Voice] = &[
    // —— 女声（经典/普通）——
    Voice { vcn: "xiaoyan", name: "小燕", gender: "female", type_: "classic", tag: "标准女声，默认" },
    Voice { vcn: "xiaoqi", name: "小琪", gender: "female", type_: "classic", tag: "经典女声" },
    Voice { vcn: "aisxping", name: "小萍", gender: "female", type_: "classic", tag: "女声" },
    Voice { vcn: "aisjinger", name: "小婧", gender: "female", type_: "classic", tag: "女声" },
    Voice { vcn: "vixy", name: "vixy", gender: "female", type_: "classic", tag: "女声·中英" },
    Voice { vcn: "vimeiyu", name: "vimeiyu", gender: "female", type_: "classic", tag: "女声" },
    Voice { vcn: "vixying", name: "vixying", gender: "female", type_: "classic", tag: "女声" },
    Voice { vcn: "vixx", name: "vixx", gender: "female", type_: "classic", tag: "女声" },
    Voice { vcn: "catherine", name: "catherine", gender: "female", type_: "classic", tag: "英文女声，中文不出声" },
    Voice { vcn: "mary", name: "mary", gender: "female", type_: "classic", tag: "英文女声，中文不出声" },
    // —— 男声（经典/普通）——
    Voice { vcn: "xiaoyu", name: "小宇", gender: "male", type_: "classic", tag: "男声" },
    Voice { vcn: "xiaofeng", name: "小峰", gender: "male", type_: "classic", tag: "男声" },
    Voice { vcn: "aisjiuxu", name: "久许", gender: "male", type_: "classic", tag: "男声" },
    Voice { vcn: "vinn", name: "vinn", gender: "male", type_: "classic", tag: "男声" },
    // —— 极速拟人（x5/x6，跨性别）——
    Voice { vcn: "x6_dongmanshaonv_pro", name: "动漫少女", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaoyue_pro", name: "聆小玥", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingyuyan_pro", name: "聆玉言", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x5_lingxiaotang_flow", name: "聆小糖", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaoxuan_pro", name: "聆小璇", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x5_lingyuzhao_flow", name: "聆玉昭", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaoying_pro", name: "聆小颖", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaozhen_pro", name: "聆小瑱", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_ganliannvxing_pro", name: "干练女性", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingyufei_pro", name: "聆玉菲", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_pangbainv1_pro", name: "旁白女声", gender: "female", type_: "x6", tag: "旁白" },
    Voice { vcn: "x6_lingxiaoyun_pro", name: "聆小芸", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingyuaner_pro", name: "聆园儿", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaoshan_pro", name: "聆小珊", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_lingxiaoli_pro", name: "聆小璃", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_xiaoqiChat_pro", name: "聆小琪", gender: "female", type_: "x6", tag: "交互 · 情感女声" },
    Voice { vcn: "x6_cuishounvsheng_pro", name: "催收女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_yingxiaonv_pro", name: "营销女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_shibingnvsheng_mini", name: "士兵女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_kongbunvsheng_mini", name: "恐怖女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_yulexinwennvsheng_mini", name: "娱乐新闻女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_jingqudaolannvsheng_mini", name: "景区导览女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_wumeinv_pro", name: "妩媚姐姐", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_huajidama_pro", name: "滑稽大妈", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_lingxiaoxue_pro", name: "聆小雪", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_gufengxianv_mini", name: "古风侠女", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_wuyediantai_mini", name: "午夜电台", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_zhuanyenvzhuchi_pro", name: "大会主持女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_ranzhinvdazi_pro", name: "运动陪练女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_zhantingnvjiedai_pro", name: "展厅接待女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_huifangnv_pro", name: "回访女声", gender: "female", type_: "x6", tag: "交互 · 成年女声" },
    Voice { vcn: "x6_dudulibao_pro", name: "少女可莉", gender: "female", type_: "x6", tag: "女声" },
    Voice { vcn: "x6_lingyouyou_pro", name: "聆佑佑", gender: "female", type_: "x6", tag: "女童" },
    Voice { vcn: "x6_lingfeiyi_pro", name: "聆飞逸", gender: "male", type_: "x6", tag: "交互 · 成熟男声" },
    Voice { vcn: "x6_lingfeibo_pro", name: "聆飞博", gender: "male", type_: "x6", tag: "交互 · 成熟男声" },
    Voice { vcn: "x6_gaolengnanshen_pro", name: "高冷男神", gender: "male", type_: "x6", tag: "交互 · 成熟男声" },
    Voice { vcn: "x6_waiguodashu_pro", name: "外国人大叔", gender: "male", type_: "x6", tag: "交互 · 成熟男声" },
    Voice { vcn: "x6_gufengpangbai_pro", name: "古风旁白", gender: "male", type_: "x6", tag: "旁白 · 成熟男声" },
    Voice { vcn: "x6_pangbainan1_pro", name: "旁白男声", gender: "male", type_: "x6", tag: "旁白 · 成熟男声" },
    Voice { vcn: "x6_lingfeihan_pro", name: "聆飞瀚", gender: "male", type_: "x6", tag: "旁白 · 成熟男声" },
    Voice { vcn: "x6_lingfeihao_pro", name: "聆飞皓", gender: "male", type_: "x6", tag: "旁白 · 成熟男声" },
    Voice { vcn: "x6_ruyadashu_pro", name: "儒雅大叔", gender: "male", type_: "x6", tag: "旁白 · 成熟男声" },
    Voice { vcn: "x6_feizheChat_pro", name: "聆飞哲", gender: "male", type_: "x6", tag: "交互 · 成熟男声" },
    Voice { vcn: "x6_wennuancixingnansheng_mini", name: "温暖磁性男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_xiaonaigoudidi_mini", name: "小奶狗弟弟", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_wenrounansheng_mini", name: "温柔男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_daqixuanchuanpiannansheng_mini", name: "大气宣传片男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_xiangruiyingyu_pro", name: "商务殷语", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_taiqiangnuannan_pro", name: "台湾腔温柔男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_lingbosong_pro", name: "聆伯松", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_huoposhaonian_pro", name: "活泼少年", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_tiexinnanyou_mini", name: "贴心男友", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_youxinanshibing_pro", name: "士兵男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_zuixianlibai_pro", name: "醉仙李白", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_bokenansheng_pro", name: "播客男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_zhuanyenanzhuchi_pro", name: "大会主持男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_zhantingnanjiedai_pro", name: "展厅接待男声", gender: "male", type_: "x6", tag: "交互 · 成年男声" },
    Voice { vcn: "x6_huanlemianbao_pro", name: "海绵宝宝", gender: "male", type_: "x6", tag: "男声" },
    Voice { vcn: "x6_shiwangxiaoxin_pro", name: "奶凶辛巴", gender: "male", type_: "x6", tag: "男童" },
];

/// GET /api/tts/voices：全量目录（客户端按 gender 分组、type 过滤建下拉）。
async fn voices() -> Response {
    Json(serde_json::json!({ "voices": VOICES })).into_response()
}

/// 音色目录子路由（并入主 router）。
pub fn router() -> Router<std::sync::Arc<crate::engine::Engines>> {
    Router::new().route("/api/tts/voices", get(voices))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录契约：vcn 唯一、gender/type 合法、总量与实测数一致。
    #[test]
    fn catalog_is_unique_and_grouped() {
        let mut vcns = std::collections::HashSet::new();
        for v in VOICES {
            assert!(vcns.insert(v.vcn), "重复 vcn: {}", v.vcn);
            assert!(matches!(v.gender, "female" | "male"), "非法性别: {}", v.vcn);
            assert!(matches!(v.type_, "classic" | "x6"), "非法类型: {}", v.vcn);
        }
        assert_eq!(VOICES.len(), 73);
        let j = serde_json::to_value(voices_payload()).unwrap();
        assert_eq!(j["voices"].as_array().unwrap().len(), 73);
        assert_eq!(j["voices"][0]["vcn"], "xiaoyan");
        assert_eq!(j["voices"][0]["gender"], "female");
        assert_eq!(j["voices"][0]["type"], "classic");
    }

    fn voices_payload() -> serde_json::Value {
        serde_json::json!({ "voices": VOICES })
    }
}
