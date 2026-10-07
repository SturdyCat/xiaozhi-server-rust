package com.xiaozhi.admin

/**
 * Kokoro voices.bin 的 sid ↔ 音色名目录（基于 sherpa-onnx kokoro-int8-multi-lang-v1_1
 * 的 voices.bin 生成脚本 `scripts/kokoro/v1.1-zh/generate_voices_bin.py` 的顺序：
 *   0-2: af_maple/af_sol/bf_vale（英文女声）
 *   3-57: zf_xxx（中文女声，55 个；列表来自 sherpa 官方 sid 表）
 *   58-102: zm_xxx（中文男声，45 个）
 * ⚠️ 顺序由 voices.bin 决定，不能自行排序——sid 必须与 voices.bin 索引一致，否则选错音色。
 */
object VoiceCatalog {
    /** 英文女声（sid 0-2） */
    val EN_FEMALE: List<VoiceOption> = listOf(
        VoiceOption("af_maple", 0),
        VoiceOption("af_sol", 1),
        VoiceOption("bf_vale", 2),
    )

    val ZF_NAMES: List<String> = listOf(
        "zf_001", "zf_002", "zf_003", "zf_004", "zf_005", "zf_006",
        "zf_007", "zf_008", "zf_017", "zf_018", "zf_019", "zf_021",
        "zf_022", "zf_023", "zf_024", "zf_026", "zf_027", "zf_028",
        "zf_032", "zf_036", "zf_038", "zf_039", "zf_040", "zf_042",
        "zf_043", "zf_044", "zf_046", "zf_047", "zf_048", "zf_049",
        "zf_051", "zf_059", "zf_060", "zf_067", "zf_070", "zf_071",
        "zf_072", "zf_073", "zf_074", "zf_075", "zf_076", "zf_077",
        "zf_078", "zf_079", "zf_083", "zf_084", "zf_085", "zf_086",
        "zf_087", "zf_088", "zf_090", "zf_092", "zf_093", "zf_094",
        "zf_099",
    )

    val ZM_NAMES: List<String> = listOf(
        "zm_009", "zm_010", "zm_011", "zm_012", "zm_013", "zm_014",
        "zm_015", "zm_016", "zm_020", "zm_025", "zm_029", "zm_030",
        "zm_031", "zm_033", "zm_034", "zm_035", "zm_037", "zm_041",
        "zm_045", "zm_050", "zm_052", "zm_053", "zm_054", "zm_055",
        "zm_056", "zm_057", "zm_058", "zm_061", "zm_062", "zm_063",
        "zm_064", "zm_065", "zm_066", "zm_068", "zm_069", "zm_080",
        "zm_081", "zm_082", "zm_089", "zm_091", "zm_095", "zm_096",
        "zm_097", "zm_098", "zm_100",
    )

    /** 中文女声（sid 3 起连续） */
    val ZH_FEMALE: List<VoiceOption> = ZF_NAMES.mapIndexed { i, n -> VoiceOption(n, 3 + i) }

    /** 中文男声（sid 58 起连续） */
    val ZH_MALE: List<VoiceOption> = ZM_NAMES.mapIndexed { i, n -> VoiceOption(n, 58 + i) }

    fun options(gender: String): List<VoiceOption> = when (gender) {
        "male" -> ZH_MALE
        "en" -> EN_FEMALE
        else -> ZH_FEMALE
    }

    fun sidOf(id: String): Int? =
        (EN_FEMALE + ZH_FEMALE + ZH_MALE).firstOrNull { it.id == id }?.sid

    /** 性别筛选项（下拉）：中文女声 / 中文男声 / 英文女声 */
    val GENDERS: List<Pair<String, String>> = listOf(
        "female" to "中文女声",
        "male" to "中文男声",
        "en" to "英文女声",
    )
}

data class VoiceOption(val id: String, val sid: Int)
