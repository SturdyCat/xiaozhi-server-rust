/**
 * `[tts]` 语音合成配置段（W6 / Config C3）。
 *
 * 字段清单与默认值：`plugins/tts/mod.rs:69-127`（`TtsConfig` / `LEGACY_BODY_FIELDS`）、
 * `:145-166`（`KokoroTtsConfig`）、`plugins/tts/xfyun.rs:16-36`（`XfyunTtsConfig`）、
 * `client/shared/.../ConfigFormFill.kt:59-97`（旧 Kuikly 表单口径）。
 * advanced 归属：`plugins/registry/projection.rs:31-40`
 * （`tts.cache_entries` + `tts.kokoro.*` 8 项，全部来自 `ADVANCED_KEYS`；
 * 简洁模式二次确认由 `ConfigPage` 的 `dirtyPaths ∩ advanced_fields` 自动覆盖）。
 *
 * 私有段按 `tts.engine` **条件渲染**（对齐 `ConfigStageCards.kt:209/314`）：
 * `[tts.kokoro]` 与 `[tts.xfyun]` 在配置里始终并存，服务端只校验被选中的实现，
 * 旧段残留是预期且无害 —— 因此绝不把未选中实现的 presence / 字段当生效值展示。
 *
 * `tts` 不在 `APP_SECTION_HOT`，`SectionHotHint` 取不到就返回 null，不自造档位文案；
 * 段内提到档位时统一复用 `SaveBar.HOT_LABEL`。
 *
 * 音色目录端点 `/api/tts/voices*` 仅在 `#[cfg(feature = "native-plugins")]` 挂载
 * （`server/src/gateway/ws/mod.rs:244-250`）；未编入时 GET 会被 SPA fallback 接走返回
 * **HTML 200**。`catalogueApi.getTtsVoices()` 对此 fail-closed：形状不符抛
 * `CatalogueUnavailableError`，这里用 `UnavailableNotice` 呈现，绝不伪装成空目录。
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { useRequest } from "ahooks";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { HOT_LABEL } from "@/components/config/SaveBar";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SecretField } from "@/components/form/SecretField";
import { SelectField } from "@/components/form/SelectField";
import { TextField } from "@/components/form/TextField";
import { toText, type SelectOption } from "@/components/form/types";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import {
  catalogueErrorMessage,
  getTtsVoices,
  isCatalogueUnavailable,
  refreshTtsVoices,
  testTtsCredentials,
  type CredentialsTestResult,
  type Voice,
  type VoicesResult,
} from "@/lib/catalogueApi";

/** 实现 id 选项；未知值由 SelectField 只告警、不纠正。 */
const ENGINE_OPTIONS: SelectOption[] = [
  { value: "kokoro", label: "本地 Kokoro INT8（离线）" },
  { value: "xfyun", label: "科大讯飞（在线）" },
  { value: "wasm-policy", label: "WASM 控制策略 + 原生 Kokoro" },
];

/** Kokoro 语言选项（模型创建时固定，plugins/tts/mod.rs:161-162）。 */
const KOKORO_LANG_OPTIONS: SelectOption[] = [
  { value: "zh", label: "中文（zh）" },
  { value: "en", label: "英文（en）" },
];

/** 轮询参数对齐旧 Kuikly `ConfigFormCatalogue.kt:169-187`：2s 间隔、90 次上限。 */
const POLL_INTERVAL_MS = 2000;
const POLL_MAX_ATTEMPTS = 90;

/** 音色下拉里的「自定义」哨兵值（Radix Select 不允许空串 item value）。 */
const CUSTOM_VOICE = "__custom__";

const GENDER_LABEL: Record<string, string> = {
  female: "女声",
  male: "男声",
};
const TYPE_LABEL: Record<string, string> = {
  classic: "普通发音人",
  x6: "极速拟人",
};

interface VoiceGroup {
  key: string;
  label: string;
  items: Voice[];
}

/** 按 gender × type 分组，已知组合固定顺序，未知组合按 key 排序追加。 */
function groupVoices(voices: Voice[]): VoiceGroup[] {
  const preferred = ["female|classic", "female|x6", "male|classic", "male|x6"];
  const buckets = new Map<string, Voice[]>();
  for (const voice of voices) {
    const key = `${voice.gender}|${voice.type}`;
    const bucket = buckets.get(key);
    if (bucket !== undefined) {
      bucket.push(voice);
    } else {
      buckets.set(key, [voice]);
    }
  }
  // 未知组合按 key 升序追加；es-toolkit sortBy 类型仅支持 object[]（T extends object），
  // 纯 string[] 无库函数覆盖，保留原生 sort（UTF-16 code-unit 升序）。
  const rest = [...buckets.keys()].filter((key) => !preferred.includes(key)).sort();
  const keys = [...preferred.filter((key) => buckets.has(key)), ...rest];
  return keys.map((key) => {
    const parts = key.split("|");
    const gender = parts[0] ?? "";
    const type = parts[1] ?? "";
    return {
      key,
      label: `${GENDER_LABEL[gender] ?? gender} · ${TYPE_LABEL[type] ?? type}`,
      items: buckets.get(key) ?? [],
    };
  });
}

/** `[tts.kokoro]` 私有段：8 个字段全部 advanced。 */
function KokoroSection(): ReactElement {
  return (
    <SectionCard
      title="Kokoro 本地合成参数"
      description="仅当 tts.engine = kokoro（或留空走服务端默认）时生效；字段来自 [tts.kokoro]（plugins/tts/mod.rs:145-166）。"
    >
      <div className="divide-y divide-border">
        <TextField
          path="tts.kokoro.model"
          label="模型文件"
          advanced
          placeholder="/data/models/kokoro/model.onnx"
          hint="Kokoro INT8 ONNX 模型路径（plugins/tts/mod.rs:149-150）。"
        />
        <TextField
          path="tts.kokoro.voices"
          label="音色向量表"
          advanced
          placeholder="/data/models/kokoro/voices.bin"
          hint="voices 二进制路径，必须与模型版本配套（plugins/tts/mod.rs:151-152）。"
        />
        <TextField
          path="tts.kokoro.tokens"
          label="Tokens 词表"
          advanced
          hint="默认随模型目录；留空由服务端默认值兜底（plugins/tts/mod.rs:153-154）。"
        />
        <TextField
          path="tts.kokoro.data_dir"
          label="数据目录"
          advanced
          hint="espeak-ng 等运行时数据目录（plugins/tts/mod.rs:155-156）。"
        />
        <TextField
          path="tts.kokoro.dict_dir"
          label="词典目录"
          advanced
          hint="发音词典目录（plugins/tts/mod.rs:157-158）。"
        />
        <TextField
          path="tts.kokoro.lexicon"
          label="词典文件"
          advanced
          hint="lexicon 文件路径（plugins/tts/mod.rs:159-160）。"
        />
        <SelectField
          path="tts.kokoro.lang"
          label="语言"
          advanced
          options={KOKORO_LANG_OPTIONS}
          hint="模型创建时固定，中文场景用 zh；服务端返回未知值时界面不自动纠正（plugins/tts/mod.rs:161-162）。"
        />
        <NumberField
          path="tts.kokoro.num_threads"
          label="合成线程数"
          advanced
          min={1}
          placeholder="4"
          hint="u32，默认 4（ConfigFormFill.kt:83）；与 ASR 抢 CPU 时可适当调小。"
        />
      </div>
    </SectionCard>
  );
}

/** `[tts.xfyun]` 私有段：凭据自检 + 音色目录探测（只在 engine=xfyun 时挂载）。 */
function XfyunSection(): ReactElement {
  const { getField, getSecret, setField } = useConfigDraft();
  const appId = toText(getField("tts.xfyun.app_id")).trim();
  const hasApiKey = getField("tts.xfyun.has_api_key") === true;
  const hasApiSecret = getField("tts.xfyun.has_api_secret") === true;
  const credsReady = appId !== "" && hasApiKey && hasApiSecret;
  const currentVoice = toText(getField("tts.xfyun.voice"));

  const [credBusy, setCredBusy] = useState(false);
  const [credResult, setCredResult] = useState<CredentialsTestResult | null>(null);
  const [credError, setCredError] = useState<string | null>(null);

  const [voices, setVoices] = useState<VoicesResult | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [unavailableMessage, setUnavailableMessage] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const [probing, setProbing] = useState(false);
  const [pollAttempt, setPollAttempt] = useState(0);
  const [refreshMessage, setRefreshMessage] = useState<string | null>(null);
  const [refreshError, setRefreshError] = useState<string | null>(null);

  const [voiceMode, setVoiceMode] = useState<"catalogue" | "custom">("catalogue");

  const mountedRef = useRef(true);
  const pollSeqRef = useRef(0);
  /** 首跳延迟 timer（对齐旧 pollOnce 首次 setTimeout）；续轮询由 ahooks 接管。 */
  const pollTimerRef = useRef<number | null>(null);
  /** 轮询请求所属的 pollSeq；新一轮受理后旧响应按此作废。 */
  const probeRunSeqRef = useRef(0);
  /** 已收到的 probing 响应计数（对齐旧 attempt 递增口径）。 */
  const probeAttemptRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      pollSeqRef.current += 1;
      if (pollTimerRef.current !== null) {
        window.clearTimeout(pollTimerRef.current);
        pollTimerRef.current = null;
      }
    };
  }, []);

  /** 读取一次目录；fail-closed 错误转成 UnavailableNotice 状态，不伪装空目录。 */
  const loadVoices = useCallback(async (): Promise<VoicesResult | null> => {
    setLoading(true);
    try {
      const result = await getTtsVoices();
      if (!mountedRef.current) {
        return null;
      }
      setVoices(result);
      setUnavailable(false);
      setUnavailableMessage(null);
      setLoadError(null);
      return result;
    } catch (error) {
      if (!mountedRef.current) {
        return null;
      }
      if (isCatalogueUnavailable(error)) {
        setVoices(null);
        setUnavailable(true);
        setUnavailableMessage(catalogueErrorMessage(error));
        setLoadError(null);
      } else {
        setLoadError(catalogueErrorMessage(error));
      }
      return null;
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  // 三要素齐备才尝试读目录；依赖变化（保存后 has_* 由回读刷新）会重新拉取。
  useEffect(() => {
    if (credsReady) {
      void loadVoices();
    }
  }, [credsReady, loadVoices]);

  /** 测试凭据：只测不存，空/缺字段由服务端逐项回落盘上凭据。 */
  const handleTestCredentials = (): void => {
    setCredBusy(true);
    setCredResult(null);
    setCredError(null);
    const body = {
      app_id: toText(getField("tts.xfyun.app_id")).trim(),
      api_key: (getSecret("tts.xfyun.api_key") ?? "").trim(),
      api_secret: (getSecret("tts.xfyun.api_secret") ?? "").trim(),
      voice: toText(getField("tts.xfyun.voice")).trim(),
    };
    void testTtsCredentials(body)
      .then((result) => {
        if (mountedRef.current) {
          setCredResult(result);
        }
      })
      .catch((error: unknown) => {
        if (mountedRef.current) {
          setCredError(catalogueErrorMessage(error));
        }
      })
      .finally(() => {
        if (mountedRef.current) {
          setCredBusy(false);
        }
      });
  };

  /**
   * 轮询 `GET /api/tts/voices` 直到 `probing === false`，然后重拉目录作为最终状态。
   * useRequest pollingInterval 接管续轮询与卸载清理外的调度；首跳仍延后 2s（对齐旧
   * pollOnce 的首次 setTimeout），过期响应用 pollSeq 作废（对齐旧 seq 守卫）。
   */
  const probeRequest = useRequest(getTtsVoices, {
    manual: true,
    pollingInterval: POLL_INTERVAL_MS,
    // 旧版 setTimeout 轮询不感知页面可见性，保持后台继续探测。
    pollingWhenHidden: true,
    // 一次失败即停（对齐旧版 catch 不续轮询）。
    pollingErrorRetryCount: 0,
    onSuccess: (snapshot: VoicesResult) => {
      // 新一轮探测已受理（pollSeq 已递增）：本轮响应作废。cancel 让本响应不再是
      // 「最新 run」，polling onFinally 的 count 守卫不通过，不会排下次 refresh
      // （否则该轮询链会在 2s 后复活）。
      if (!mountedRef.current || pollSeqRef.current !== probeRunSeqRef.current) {
        cancelVoiceProbe();
        return;
      }
      if (snapshot.probing) {
        const next = probeAttemptRef.current + 1;
        probeAttemptRef.current = next;
        setPollAttempt(next);
        setRefreshMessage(`探测中…（第 ${next}/${POLL_MAX_ATTEMPTS} 次检查，每 2 秒）`);
        if (next >= POLL_MAX_ATTEMPTS) {
          // cancel() 递增 Fetch.count：polling 的 onFinally 不再调度下一次刷新。
          cancelVoiceProbe();
          setProbing(false);
          setRefreshMessage(null);
          setRefreshError(
            `探测超时：已轮询 ${POLL_MAX_ATTEMPTS} 次（约 ${(POLL_MAX_ATTEMPTS * POLL_INTERVAL_MS) / 1000} 秒），服务端仍在探测，请查看服务端日志。本次不视为成功。`,
          );
        }
        return;
      }
      // probing 转 false 后按 Kuikly 语义再重拉一次目录，不直接拿轮询快照冒充成功。
      cancelVoiceProbe();
      void loadVoices().then((final) => {
        if (!mountedRef.current || pollSeqRef.current !== probeRunSeqRef.current) {
          return;
        }
        setProbing(false);
        if (final === null) {
          setRefreshMessage(null);
          setRefreshError("探测已结束，但目录重拉失败；上面已显示具体原因，不视为探测成功。");
          return;
        }
        setRefreshError(null);
        if (final.source === "empty" && final.fetchedAt === 0) {
          // 尚未探测：交给下方「尚未探测」的可行动指引呈现，不报「服务端坏了」，
          // 也不写成探测成功。
          setRefreshMessage(
            "探测流程已结束，但目录仍是「尚未探测」状态；请按下方指引先保存三要素后重试，本次不视为探测成功。",
          );
        } else if (final.voices.length === 0) {
          // 探测完成但 0 条可用：由下方 warning 呈现，不假装拿到目录。
          setRefreshMessage(
            "探测完成但 0 条可用音色；请按下方提示核对账号授权，本次不视为探测成功。",
          );
        } else {
          setRefreshMessage(
            `探测完成：共 ${final.voices.length} 个实测可用音色，已按性别 × 类型分组。`,
          );
        }
      });
    },
    onError: (error: unknown) => {
      if (!mountedRef.current || pollSeqRef.current !== probeRunSeqRef.current) {
        // 同 onSuccess：拒绝分支必须 cancel，防止轮询链复活。
        cancelVoiceProbe();
        return;
      }
      setProbing(false);
      setRefreshMessage(null);
      if (isCatalogueUnavailable(error)) {
        setUnavailable(true);
        setUnavailableMessage(catalogueErrorMessage(error));
        setVoices(null);
      }
      setRefreshError(`轮询音色目录失败：${catalogueErrorMessage(error)}`);
    },
  });
  const { run: runVoiceProbe, cancel: cancelVoiceProbe } = probeRequest;

  // 卸载停轮询：ahooks 内置 useUnmount(() => cancel())，此显式 cleanup 为冗余保险；
  // 首跳延迟 timer 由上方既有 unmount 清理（ahooks 不管外部 setTimeout）。
  useEffect(() => {
    return () => {
      cancelVoiceProbe();
    };
  }, [cancelVoiceProbe]);

  /** 启动后台探测：立即 200 只是「已受理」，后续靠轮询确认结束。 */
  const handleRefresh = (): void => {
    // 入口先 cancel：作废在途响应（count++）+ 清 pending 轮询 timer（onCancel）。
    // 不加的话，旧链在「pollSeq 已递增、probeRunSeqRef 尚未更新为新 seq」的窗口外，
    // 还存在「probeRunSeqRef 已更新但旧链响应先到」的窗口——两 ref 相等守卫失效，
    // 旧快照会被当作新一轮检查、probeAttempt 计数漂移，僵尸链 2s 后复活。
    cancelVoiceProbe();
    pollSeqRef.current += 1;
    const seq = pollSeqRef.current;
    probeAttemptRef.current = 0;
    setProbing(true);
    setPollAttempt(0);
    setRefreshError(null);
    setRefreshMessage("正在启动后台探测…");
    void refreshTtsVoices()
      .then((started) => {
        if (!mountedRef.current || seq !== pollSeqRef.current) {
          return;
        }
        setRefreshMessage(`探测已启动（候选 ${started.candidates} 个），后台逐个真实合成验证中…`);
        // 首跳延后 2s（对齐旧 pollOnce 首次 setTimeout）；之后由 pollingInterval 接管。
        probeRunSeqRef.current = seq;
        pollTimerRef.current = window.setTimeout(() => {
          pollTimerRef.current = null;
          if (!mountedRef.current || seq !== pollSeqRef.current) {
            return;
          }
          runVoiceProbe();
        }, POLL_INTERVAL_MS);
      })
      .catch((error: unknown) => {
        if (!mountedRef.current || seq !== pollSeqRef.current) {
          return;
        }
        setProbing(false);
        setRefreshMessage(null);
        if (isCatalogueUnavailable(error)) {
          setVoices(null);
          setUnavailable(true);
          setUnavailableMessage(catalogueErrorMessage(error));
        }
        setRefreshError(`启动探测失败：${catalogueErrorMessage(error)}`);
      });
  };

  const groups = voices === null ? [] : groupVoices(voices.voices);
  const knownVoice = voices !== null && voices.voices.some((voice) => voice.vcn === currentVoice);
  const selectValue = voiceMode === "custom" ? CUSTOM_VOICE : knownVoice ? currentVoice : "";
  const neverProbed = voices !== null && voices.source === "empty" && voices.fetchedAt === 0;

  return (
    <div className="space-y-4">
      <FormNotice tone="warning">
        讯飞三要素中只有 app_id 是明文回传的普通字段：留空保存会把它清空，不是「留空保留」。api_key
        / api_secret 属于服务端
        SECRET_PATHS（server/src/gateway/ws/config.rs:74-86），走密钥控件，留空 = 保留原值。
      </FormNotice>

      <SectionCard
        title="讯飞在线合成凭据"
        description="字段来自 [tts.xfyun]（plugins/tts/xfyun.rs:16-36）；凭据测试与音色探测都只读取表单/服务端已存值，不会写入配置。"
      >
        <div className="divide-y divide-border">
          <TextField
            path="tts.xfyun.app_id"
            label="应用 AppID"
            placeholder="讯飞开放平台 APPID"
            hint="明文非密钥，留空会被清空，不是留空保留；在讯飞「在线语音合成」控制台创建应用后取得。"
          />
          <SecretField path="tts.xfyun.api_key" hasPath="tts.xfyun.has_api_key" label="API Key" />
          <SecretField
            path="tts.xfyun.api_secret"
            hasPath="tts.xfyun.has_api_secret"
            label="API Secret"
          />
          <TextField
            path="tts.xfyun.voice"
            label="发音人 vcn"
            placeholder="xiaoyan"
            hint="默认 xiaoyan（plugins/tts/xfyun.rs:29-36）；可由下方音色下拉回填，也可直接手填。"
          />

          <div className="grid gap-1.5 py-2">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={credBusy}
                onClick={handleTestCredentials}
              >
                {credBusy ? (
                  <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                ) : null}
                测试凭据
              </Button>
              <span className="text-sm font-medium">
                只测不存：留空的字段由服务端逐项回落到盘上已存凭据
              </span>
            </div>
            {credResult !== null ? (
              <div className="pt-1">
                <FormNotice tone={credResult.ok ? "info" : "danger"}>
                  {credResult.ok
                    ? `凭据可用（合成耗时 ${credResult.elapsedMs} ms）。`
                    : `凭据不可用：${credResult.error}`}
                </FormNotice>
              </div>
            ) : null}
            {credError !== null ? (
              <div className="pt-1">
                <FormNotice tone="danger">凭据测试失败：{credError}</FormNotice>
              </div>
            ) : null}
          </div>
        </div>
      </SectionCard>

      {credsReady ? (
        <SectionCard
          title="音色目录"
          description="目录来自服务端对候选音色的真实合成探测（server/src/device/voices.rs:1-17），只有当前账号实际可用的发音人才会列出。"
        >
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={probing || loading}
                onClick={handleRefresh}
              >
                {probing || loading ? (
                  <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                ) : null}
                探测音色目录
              </Button>
              {refreshMessage !== null ? (
                <span className="text-sm font-medium">{refreshMessage}</span>
              ) : null}
              {probing ? (
                <span className="text-sm font-medium">
                  已检查 {pollAttempt}/{POLL_MAX_ATTEMPTS} 次
                </span>
              ) : null}
            </div>

            {unavailable ? (
              <>
                <UnavailableNotice
                  title="音色目录能力未启用"
                  description={
                    unavailableMessage ??
                    "服务端未挂载 /api/tts/voices*（通常是构建未启用 native-plugins）；该状态下不能探测或选择音色，配置字段仍可正常保存。"
                  }
                />
                <FormNotice tone="warning">
                  音色目录端点未启用或被管理页 HTML
                  回退接走：现在不能探测、也不能从下拉选择音色，但上方 [tts]
                  配置字段仍可正常保存。请改用启用 native-plugins 的服务端构建，或直接在「发音人
                  vcn」手填。
                </FormNotice>
              </>
            ) : null}

            {loadError !== null ? (
              <FormNotice tone="danger">读取音色目录失败：{loadError}</FormNotice>
            ) : null}
            {refreshError !== null ? <FormNotice tone="danger">{refreshError}</FormNotice> : null}

            {neverProbed ? (
              <FormNotice tone="warning">
                音色目录尚未探测（source=empty、fetched_at=0）：请先保存讯飞三要素，再点「探测音色目录」，服务端会对候选音色逐个真实合成验证（约需
                1 分钟）。这不是服务端故障，也不代表探测已完成。
              </FormNotice>
            ) : null}

            {voices !== null && !neverProbed && voices.voices.length === 0 ? (
              <FormNotice tone="warning">
                探测已完成，但当前账号没有可用音色（0
                条）：请确认讯飞控制台已开通在线语音合成并授权发音人，必要时重新探测。这不视为探测出可用目录。
              </FormNotice>
            ) : null}

            {voices !== null && voices.voices.length > 0 ? (
              <div className="grid gap-1.5">
                <p className="text-sm font-medium">
                  从音色目录选择（按性别 × 类型分组，共 {voices.voices.length} 条）
                </p>
                <Select
                  value={selectValue}
                  onValueChange={(next) => {
                    if (next === CUSTOM_VOICE) {
                      setVoiceMode("custom");
                      return;
                    }
                    setVoiceMode("catalogue");
                    setField("tts.xfyun.voice", next);
                  }}
                >
                  <SelectTrigger className="w-[320px]">
                    <SelectValue placeholder="请选择发音人" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      <SelectLabel>自定义</SelectLabel>
                      <SelectItem value={CUSTOM_VOICE}>自定义（手填 vcn）</SelectItem>
                    </SelectGroup>
                    {groups.map((group) => (
                      <SelectGroup key={group.key}>
                        <SelectLabel>{group.label}</SelectLabel>
                        {group.items.map((voice) => (
                          <SelectItem key={voice.vcn} value={voice.vcn}>
                            {voice.name}
                            {voice.tag !== "" ? `（${voice.tag}）` : ""} · {voice.vcn}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    ))}
                  </SelectContent>
                </Select>
                {voiceMode === "custom" ? (
                  <FormNotice tone="info">
                    自定义模式：请在上方「发音人 vcn」输入框直接手填讯飞控制台上的标识（如
                    x4_yezi）。服务端不会校验是否存在，填错会在合成时失败。
                  </FormNotice>
                ) : null}
              </div>
            ) : null}
          </div>
        </SectionCard>
      ) : (
        <FormNotice tone="warning">
          请先保存讯飞三要素（app_id / api_key / api_secret），保存成功后本页回读到 has_api_key /
          has_api_secret
          均为已配置，才会出现「探测音色目录」按钮与音色下拉。当前三要素未齐备（app_id
          非空、has_api_key、has_api_secret
          三项须同时满足），界面暂不提供音色区，避免选出必然失败的发音人；这不是报错，也不代表目录可用。
        </FormNotice>
      )}
    </div>
  );
}

export function TtsSection(): ReactElement {
  const { getField } = useConfigDraft();
  const engine = toText(getField("tts.engine"));
  const knownEngine = ENGINE_OPTIONS.some((option) => option.value === engine);
  const showKokoro = engine === "" || engine === "kokoro";
  const showXfyun = engine === "xfyun";
  const showWasm = engine === "wasm-policy";

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="tts" />

      <SectionCard
        title="TTS · 语音合成（公共字段）"
        description="跨实现通用参数；字段来自 [tts]（plugins/tts/mod.rs:69-96）。实现私有参数按下方 engine 条件展示。"
      >
        <div className="divide-y divide-border">
          <SelectField
            path="tts.engine"
            label="实现 id"
            options={ENGINE_OPTIONS}
            hint="kokoro（本地，默认）| xfyun（讯飞在线）| wasm-policy（WASM 控制策略 + 原生 Kokoro）。服务端返回未知值时界面只告警、不自动纠正。"
          />

          <NumberField
            path="tts.cache_entries"
            label="合成结果缓存条目数"
            advanced
            min={0}
            placeholder="256"
            hint={`u32，默认 256（ConfigFormFill.kt:87）；0 = 关闭缓存。该字段 ${HOT_LABEL.restart_only}，同段其余字段 ${HOT_LABEL.next_session}，一次保存的响应档位取最严格。`}
          />

          <NumberField
            path="tts.speed"
            label="语速倍率"
            integer={false}
            placeholder="1.0"
            hint="f32，默认 1.0（plugins/tts/mod.rs:85-86、ConfigFormFill.kt:86）；走小数通道，0.7 / 1.0 可直接写入。"
          />

          <NumberField
            path="tts.speaker"
            label="发音人参数槽（sid）"
            placeholder="0"
            hint="i32，默认 0（plugins/tts/mod.rs:88-89、ConfigFormFill.kt:85）；讯飞实现忽略此值，改由 tts.xfyun.voice 决定。"
          />

          <NumberField
            path="tts.max_concurrent"
            label="同时合成路上限"
            min={1}
            placeholder="1"
            hint="u32，默认 1（plugins/tts/mod.rs:90-96）；多设备同时说话时在闸门处排队，而不是压垮 CPU。"
          />
        </div>
      </SectionCard>

      {showKokoro ? <KokoroSection /> : null}
      {showXfyun ? <XfyunSection /> : null}

      {showWasm ? (
        <FormNotice tone="info">
          当前实现 wasm-policy 没有私有配置字段，只用上方公共字段；保存按最小叶子补丁提交。
        </FormNotice>
      ) : null}

      {!knownEngine && engine !== "" ? (
        <FormNotice tone="warning">
          未识别的实现 id「{engine}」：界面不展示任何私有段（避免把 [tts.kokoro] / [tts.xfyun]
          的残留字段当生效值），也不会自动改写；如不主动选择，保存时会原样提交。
        </FormNotice>
      ) : null}
    </div>
  );
}
