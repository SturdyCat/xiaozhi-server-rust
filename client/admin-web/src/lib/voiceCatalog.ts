/**
 * Kokoro 音色目录（逐字移植 `client/shared/.../admin/VoiceCatalog.kt`）。
 *
 * sid ↔ 音色名映射基于 sherpa-onnx kokoro-int8-multi-lang-v1_1 的 voices.bin 生成脚本
 * `scripts/kokoro/v1.1-zh/generate_voices_bin.py` 的顺序：
 *   0-2: af_maple/af_sol/bf_vale（英文女声）
 *   3-57: zf_xxx（中文女声，55 个）
 *   58-102: zm_xxx（中文男声，45 个）
 * ⚠️ 顺序由 voices.bin 决定，不能自行排序——sid 必须与 voices.bin 索引一致，
 * 否则选错音色（服务端按 sid 索引 voices.bin）。
 */

/** 音色选项（id + voices.bin 索引）。 */
export interface VoiceOption {
  readonly id: string;
  readonly sid: number;
}

/** 英文女声（sid 0-2）。 */
export const EN_FEMALE: readonly VoiceOption[] = [
  { id: "af_maple", sid: 0 },
  { id: "af_sol", sid: 1 },
  { id: "bf_vale", sid: 2 },
];

const ZF_NAMES: readonly string[] = [
  "zf_001",
  "zf_002",
  "zf_003",
  "zf_004",
  "zf_005",
  "zf_006",
  "zf_007",
  "zf_008",
  "zf_017",
  "zf_018",
  "zf_019",
  "zf_021",
  "zf_022",
  "zf_023",
  "zf_024",
  "zf_026",
  "zf_027",
  "zf_028",
  "zf_032",
  "zf_036",
  "zf_038",
  "zf_039",
  "zf_040",
  "zf_042",
  "zf_043",
  "zf_044",
  "zf_046",
  "zf_047",
  "zf_048",
  "zf_049",
  "zf_051",
  "zf_059",
  "zf_060",
  "zf_067",
  "zf_070",
  "zf_071",
  "zf_072",
  "zf_073",
  "zf_074",
  "zf_075",
  "zf_076",
  "zf_077",
  "zf_078",
  "zf_079",
  "zf_083",
  "zf_084",
  "zf_085",
  "zf_086",
  "zf_087",
  "zf_088",
  "zf_090",
  "zf_092",
  "zf_093",
  "zf_094",
  "zf_099",
];

const ZM_NAMES: readonly string[] = [
  "zm_009",
  "zm_010",
  "zm_011",
  "zm_012",
  "zm_013",
  "zm_014",
  "zm_015",
  "zm_016",
  "zm_020",
  "zm_025",
  "zm_029",
  "zm_030",
  "zm_031",
  "zm_033",
  "zm_034",
  "zm_035",
  "zm_037",
  "zm_041",
  "zm_045",
  "zm_050",
  "zm_052",
  "zm_053",
  "zm_054",
  "zm_055",
  "zm_056",
  "zm_057",
  "zm_058",
  "zm_061",
  "zm_062",
  "zm_063",
  "zm_064",
  "zm_065",
  "zm_066",
  "zm_068",
  "zm_069",
  "zm_080",
  "zm_081",
  "zm_082",
  "zm_089",
  "zm_091",
  "zm_095",
  "zm_096",
  "zm_097",
  "zm_098",
  "zm_100",
];

/** 中文女声（sid 3 起连续）。 */
export const ZH_FEMALE: readonly VoiceOption[] = ZF_NAMES.map((id, i) => ({
  id,
  sid: 3 + i,
}));

/** 中文男声（sid 58 起连续）。 */
export const ZH_MALE: readonly VoiceOption[] = ZM_NAMES.map((id, i) => ({
  id,
  sid: 58 + i,
}));

/** 按性别筛选音色（对齐 VoiceCatalog.options）。 */
export function voiceOptionsOf(gender: "female" | "male" | "en"): readonly VoiceOption[] {
  switch (gender) {
    case "male":
      return ZH_MALE;
    case "en":
      return EN_FEMALE;
    default:
      return ZH_FEMALE;
  }
}

/** sid 反查（对齐 VoiceCatalog.sidOf）。 */
export function sidOf(id: string): number | null {
  for (const option of [...EN_FEMALE, ...ZH_FEMALE, ...ZH_MALE]) {
    if (option.id === id) {
      return option.sid;
    }
  }
  return null;
}

/** 性别筛选项（下拉；TestBenchState.kt:82-88 + VoiceCatalog.GENDERS 逐字）。 */
export const GENDER_OPTIONS: readonly { value: "female" | "male" | "en"; label: string }[] = [
  { value: "female", label: "中文女声" },
  { value: "male", label: "中文男声" },
  { value: "en", label: "英文女声" },
];

/** 语言下拉选项（TestBenchPage.kt:219-223 LANG_OPTIONS 逐字）。 */
export const LANG_OPTIONS: readonly { value: "auto" | "zh" | "en"; label: string }[] = [
  { value: "auto", label: "自动（服务器默认）" },
  { value: "zh", label: "中文（zh）" },
  { value: "en", label: "英文（en）" },
];

/** auto → 交给服务器默认（TestBenchState.kt:548 resolveLang 逐字语义）。 */
export function resolveLang(lang: "auto" | "zh" | "en"): string {
  return lang === "auto" ? "" : lang;
}
