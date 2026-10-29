package com.xiaozhi.admin


import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.state.stateVar
import com.xiaozhi.admin.state.stateListOf
import kotlin.reflect.KMutableProperty0

/**
 * 跨端配置草稿的唯一 observable 所有者；既有卡片与 macOS 测试台共用此成员 API。
 * JSON 构建见 ConfigFormDraft，配置/overlay/预设 IO 见 ConfigFormActions，
 * 服务商目录见 ConfigFormCatalogue，只读测试请求见 ConfigFormTests。
 * 所有调用与 NetworkModule 回调沿既有 Kuikly context 线程，不自建线程。
 */
enum class NodeWorkspaceTab { Configuration, Testing }

class ConfigFormState(private val scope: PagerScope) {

    // ===== [server] =====
    var port by stateVar("8000")
    // 配置中的设备接入密钥草稿，不是当前管理请求的 Bearer token。
    var expectedToken by stateVar("")
    var workerThreads by stateVar("2")
    var adminDir by stateVar("")

    // ===== [audio] =====
    var downlinkSampleRate by stateVar("24000")
    var downlinkFrameMs by stateVar("60")
    var channels by stateVar("1")
    var binaryProtocolVersion by stateVar("1")
    /// 下行抖动缓冲提前量（毫秒）：下发时间表比实时提前该时长，使 ESP 解码队列常备音频存货，
    /// 吸收网络抖动 + 上游 LLM 出句/TTS 合成的停顿（长回复断音的来源）。
    /// ⚠️ 上限 = (downlink_queue_packets − 2) × 帧长（默认 (20−2)×60 = 1080ms；超限收敛并告警）。
    /// 默认 1000 ≈ 设备端极限（设备解码队列只有 ~1.2s，想更深必须改固件队列）。
    /// 代价：提前量越大，打断后设备把已缓存音频播完才停口。
    var downlinkLeadMs by stateVar("1000")
    /// 服务器侧下行**水库**容量（毫秒）：把已编码的 Opus 帧在服务器内存里多备几秒，
    /// 发送端从水库取帧（而不是"现合成、现发送"）→ 上游（LLM 出句 / 在线 TTS 首片）
    /// 慢于实时也不再抽干设备解码队列。0 = 关闭（逐帧直通）。内存代价极小（~2KB/s，30s ≈ 60KB）。
    /// 它同时派生 PCM **下载缓冲**预算（`clamp(本值, 1s, 10s)`，见 `src/app/prefetch.rs`）。
    /// 设备解码队列**包数**（必须与固件一致）：`20 × 帧长` 决定设备侧存货上限，
    /// 也是 `downlink_lead_ms` 安全上限的来源。自刷固件放大队列后必须同步本值。
    var downlinkQueuePackets by stateVar("20")
    var downlinkBufferMs by stateVar("30000")
    /// **起跑闸**（毫秒）：本轮首帧下发前先等水库攒够这么多音频再开播（把散在全句的
    /// 断续换成一次起跑等待）；生产端已收工的短回复立即放行，不白等。0 = 关闭。
    var downlinkBufferStartMs by stateVar("1500")
    /// 起跑闸**最长等待**（毫秒）：上游持续慢于实时时不把首帧无限期拖住。
    var downlinkBufferStartMaxMs by stateVar("4000")

    // ===== [playback] ===== 独立 NextSession 场景策略，不改动 [audio] 协商档位
    var playbackMode by stateVar("interactive")
    var announcementStartMs by stateVar("5000")
    var announcementStartMaxMs by stateVar("10000")

    // ===== [asr] =====（无 mock：ASR 恒为 SenseVoice，无 backend 字段）
    var asrModel by stateVar("")
    var asrTokens by stateVar("")
    var asrLanguage by stateVar("auto")
    var asrUseItn by stateVar("true")
    var asrNumThreads by stateVar("2")
    var asrProvider by stateVar("cpu")

    // ===== [vad] =====
    var vadModel by stateVar("")
    var vadThreshold by stateVar("0.5")
    var vadMinSilence by stateVar("0.25")
    var vadMinSpeech by stateVar("0.25")

    // ===== [tts] =====（区分「本地模型 / 远程服务」两大类，见 TTS_LOCAL/REMOTE 注册表）
    /// 当前合成方式："local"（本地模型，离线）| "remote"（远程服务，在线）。
    /// 与 [ttsBackend] 联动：切方式时 backend 跟随切到对应组的引擎/服务商。
    var ttsMode by stateVar("local")
    /// 当前生效的引擎/服务商 id（= 服务端 `[tts].engine`；本地组：kokoro…；远程组：xfyun…）。
    /// ⚠️ P4 前本地组叫 "sherpa"（旧 config.toml 的 `backend` 值），服务端仍认，这里统一为 "kokoro"。
    var ttsBackend by stateVar("kokoro")
    /// 上次使用的远程服务商（切回「远程」时恢复，默认 xfyun）。
    var lastRemoteEngine by stateVar("xfyun")
    var ttsModel by stateVar("")
    var ttsVoices by stateVar("")
    var ttsTokens by stateVar("")
    var ttsDataDir by stateVar("")
    var ttsDictDir by stateVar("")
    var ttsLexicon by stateVar("")
    var ttsLang by stateVar("zh")
    var ttsSpeaker by stateVar("0")
    var ttsSpeed by stateVar("1.0")
    var ttsNumThreads by stateVar("4")
    /// TTS 结果缓存条目数（按句缓存已编码下行 Opus 帧，跨会话共享；命中零合成延迟）。0 = 关闭。
    var ttsCacheEntries by stateVar("256")

    // ===== [tts.xfyun] =====（engine=xfyun 时使用；讯飞开放平台「在线语音合成」控制台获取）

    /** 科大讯飞三要素赋值：值变化时写入并作废只读测试凭据缓存；四个字段共用同一语义。 */
    private fun setXfyunCredential(field: KMutableProperty0<String>, value: String) {
        if (field.get() != value) {
            field.set(value)
            testRequests.invalidateCredentials()
        }
    }

    private var _xfyunAppId by stateVar("")
    var xfyunAppId: String
        get() = _xfyunAppId
        set(value) = setXfyunCredential(this::_xfyunAppId, value)
    private var _xfyunApiKey by stateVar("")
    var xfyunApiKey: String
        get() = _xfyunApiKey
        set(value) = setXfyunCredential(this::_xfyunApiKey, value)
    private var _xfyunApiSecret by stateVar("")
    var xfyunApiSecret: String
        get() = _xfyunApiSecret
        set(value) = setXfyunCredential(this::_xfyunApiSecret, value)
    /**
     * 密钥 **presence**：服务端 GET /api/config 已打码，只回 `has_api_key` / `has_api_secret`。
     * 明文永不进表单，输入框留空 = 不修改 SQLite 已保存值；为 true 时提示「已配置」。
     */
    var xfyunApiKeyConfigured by stateVar(false)
    var xfyunApiSecretConfigured by stateVar(false)
    private var _xfyunVoice by stateVar("xiaoyan")
    var xfyunVoice: String
        get() = _xfyunVoice
        set(value) = setXfyunCredential(this::_xfyunVoice, value)

    /** Non-secret catalogue scope; every reload/access/base change invalidates old polls. */
    internal var catalogueScopeEpoch = 0L
        private set

    /** 值变化时写入并执行回调（如作废测试缓存）；[serverBase]/[revision]/[editDeviceId] 共用。 */
    private fun setStringField(field: KMutableProperty0<String>, value: String, onChanged: () -> Unit) {
        if (field.get() != value) {
            field.set(value)
            onChanged()
        }
    }

    /** 拉取目录/刷新用的服务器地址（连接流程写入；空=当前页同源）。 */
    private var _serverBase by stateVar("")
    var serverBase: String
        get() = _serverBase
        set(value) = setStringField(this::_serverBase, value) { invalidateReadOnlyTests() }
    /** 上次“刷新音色目录”的结果提示（动作按钮反馈）。 */
    var voicesRefreshMsg by stateVar("")
    /** 上次“测试凭据”的结果提示（动作按钮反馈）。 */
    var credsTestMsg by stateVar("")
    var credsTestOk by stateVar(false)

    /** 配置来源提示（配置文件路径 + 持久化状态；Server 配置卡展示）。 */
    var configMetaMsg by stateVar("")

    /** 讯飞三要素是否就绪——决定依赖凭据的音色选择区是否显示。
     *  密钥可能只以 presence 形式存在（打码后表单里是空的），此时同样算就绪。 */
    fun xfyunCredsReady(): Boolean =
        xfyunAppId.isNotBlank() &&
            (xfyunApiKey.isNotBlank() || xfyunApiKeyConfigured) &&
            (xfyunApiSecret.isNotBlank() || xfyunApiSecretConfigured)

    // 音色两级选择：性别分组（女/男/自定义）× 音色类型（全部/普通/极速拟人）。
    // 音色列表经 /api/tts/voices 从服务端动态获取（服务端为唯一数据源：实测目录+控制台元数据）。
    var xfyunVoiceGroup by stateVar("female")
    val xfyunVoiceGroupOptions: MutableList<Pair<String, String>> = stateListOf()
    /// 音色类型筛选：all（全部）/ classic（普通发音人）/ x6（极速拟人）。
    var xfyunVoiceType by stateVar("all")
    val xfyunVoiceTypeOptions: MutableList<Pair<String, String>> = stateListOf()
    /** 当前分组+类型下的音色选项（切分组/切类型/拉取目录时重建；T05 起为 Compose 快照列表）。 */
    val xfyunVoiceOptions: MutableList<Pair<String, String>> = stateListOf()
    // 四个源列表（loadVoices 填充）：性别 × 类型
    val voicesFemaleClassic: MutableList<Pair<String, String>> = stateListOf()
    val voicesFemaleX6: MutableList<Pair<String, String>> = stateListOf()
    val voicesMaleClassic: MutableList<Pair<String, String>> = stateListOf()
    val voicesMaleX6: MutableList<Pair<String, String>> = stateListOf()
    /** 目录是否已拉取（拉取前按 vcn 前缀启发式分组）。 */
    var voicesLoaded by stateVar(false)

    // ===== [llm] =====（无 mock：LLM 恒为 OpenAI 兼容 HTTP，无 backend 字段）
    var llmApiBase by stateVar("")
    var llmApiKey by stateVar("")
    /** 密钥 presence（服务端打码，只回 `has_api_key`）；见 xfyun 同名说明。 */
    var llmApiKeyConfigured by stateVar(false)
    var llmModel by stateVar("")
    var llmSystemPrompt by stateVar("")
    var llmMaxHistory by stateVar("10")
    var llmTemperature by stateVar("0.7")
    var llmStream by stateVar("true")
    /// 回声开关（f68cd99 `[llm].enabled`）：false = 技能未命中不走 LLM，ASR 原文直接回声。
    /// 曾漏配 UI（对齐核对发现）：服务端+注册表齐全而管理台无字段，回声模式不可见不可切。
    var llmEnabled by stateVar("true")

    // ===== [aiui] =====（全链路极速超拟人：VAD 切段 → 讯飞 AIUI 云端 ASR+LLM+TTS 闭环）
    // ⚠️ 启用后设备流水线走云端闭环，**本地 ASR/LLM/TTS 全部闲置**（`[soul]`/`[memory]` 亦不生效）。
    var aiuiEnabled by stateVar("false")
    var aiuiAppid by stateVar("")
    var aiuiApiKey by stateVar("")
    var aiuiApiSecret by stateVar("")
    /** 密钥 presence（服务端打码，只回 `has_api_key` / `has_api_secret`）。 */
    var aiuiApiKeyConfigured by stateVar(false)
    var aiuiApiSecretConfigured by stateVar(false)
    var aiuiScene by stateVar("main_box")
    var aiuiSnPrefix by stateVar("xiaozhi")
    var aiuiVoice by stateVar("x6_dongmanshaonv_pro")
    var aiuiSpeed by stateVar("50")
    var aiuiVolume by stateVar("50")
    var aiuiPitch by stateVar("50")
    var aiuiPrompt by stateVar("")
    var aiuiPaceMs by stateVar("10")

    // ===== [soul] / [memory]：上下文生产者（拆到 ConfigContextState，见 AGENTS.md §5.10）=====
    /// 人格档案与图记忆的表单状态 + 记忆动作（状态查询/测试召回/维护/清空）。
    val sm = ContextSectionState(scope)

    // ===== [command]：指令闸门（ASR → LLM 之间；退下/闭嘴/关闭 → 断开会话）=====
    /// 拆到 `ConfigCommandState`（同上：`AGENTS.md` §5.10 文件规模约定）。
    val cmd = CommandSectionState(scope)

    // ===== [wakeup]：唤醒问候语（唤醒 → LLM 之间；唤醒即回「我在」，跳过 LLM/TTS）=====
    /// 拆到 `ConfigWakeupState`（同上：`AGENTS.md` §5.10 文件规模约定）。
    val wake = WakeupSectionState(scope)

    // ===== [nlu]：技能级联（规则 → 轻量模型 → LLM 白名单执行）=====
    /// 拆到 `ConfigNluState`（同上：`AGENTS.md` §5.10 文件规模约定）。
    val nlu = NluSectionState(scope)

    // ===== [echomuse]：EchoMuse 设备入口（enabled + token；RestartOnly）=====
    /// 拆到 `EchoMuseSectionState`（同上：`AGENTS.md` §5.10 文件规模约定）。
    val em = EchoMuseSectionState(scope)

    /// 字段元数据（`GET /api/config/schema`）：目前用于「这个字段多久生效」提示（见 FormSchemaState）。
    val schema = FormSchemaState(scope)

    // ===== 状态 =====
    /// 服务端配置内容指纹（GET /api/config 的 `revision`）：保存时回传，用于拒绝**过期写入**。
    /// 两个标签页/两个端同时打开表单时，后保存者会拿到 409 而不是静默覆盖前者的改动。
    private var _revision by stateVar("")
    var revision: String
        get() = _revision
        set(value) = setStringField(this::_revision, value) {
            testRequests.invalidate()
            nodeTests.invalidateScope()
        }
    var dirty by stateVar(false)
    var saving by stateVar(false)
    var statusMsg by stateVar("")
    var statusLevel by stateVar("info")
    var lastSavedAt by stateVar("")
    /// 上次保存因**过期写入（409）**被拒：顶栏显示「重新加载」入口（草稿保留）。
    var conflict by stateVar(false)
    /// 上次保存的「改动影响」文案（基于服务端 `hot`）：非空时在配置 section 头部提示
    /// "本次改动需：新会话生效 / 重启 server 生效"。live（立即生效）或空串则不显示。
    /// 由 save 成功时写入、load/reload 或保存失败时清空。
    var saveImpactNote by stateVar("")

    // ===== 编辑对象（P1-5：全局配置 / 设备个性化 overlay）=====
    /// 当前编辑目标："" = 全局配置；非空 = 该设备 ID 的个性化 overlay。
    /// 设备目标下 load 走「全局基线 + overlay 套用」、save 走「白名单差量 → 设备 overlay」
    ///（见 ConfigFormActions 的设备加载/保存），预设与 revision 等**全局**机制不参与。
    private var _editDeviceId by stateVar("")
    var editDeviceId: String
        get() = _editDeviceId
        set(value) = setStringField(this::_editDeviceId, value) {
            testRequests.invalidate()
            nodeTests.invalidateScope()
        }

    /**
     * 「保存配置」按钮的**完成态**（三态按钮的第三态）："" 正常 / "ok" 保存成功 / "error" 保存失败。
     * 请求进行中由 [saving] 表达（按钮文案切「保存中…」并拦截重复点击）；
     * 完成态由 ConfigFormActions 保持 2 秒后自动回落，随后由调用方的顶部 toast 接力提示。
     */
    var saveResult by stateVar("")

    /** 当前展开的下拉（TTS engine 选择等，"" = 全部收起）；放状态类避免 Pager body 重建丢失展开态。 */
    var openDropdown by stateVar("")

    // ===== 配置预设（P1-5：快照 / 应用 / 删除；仅作用于全局配置）=====
    /// 服务端已有的预设名列表（GET /api/config/presets）。
    val presetNames: MutableList<String> = stateListOf()
    /// 「保存为预设」的名称草稿。
    var presetNameDraft by stateVar("")
    /// 预设动作的就地反馈（保存 / 应用 / 删除结果；空 = 无）。
    var presetMsg by stateVar("")
    /// 预设动作进行中（拦截重复点击）。
    var presetBusy by stateVar(false)

    /** 下拉「是否展开」的判定（收敛各调用点 `openDropdown == id` 三行样板）。 */
    fun isDropdownOpen(id: String): Boolean = openDropdown == id

    /** 下拉开合切换：已展开则收起，否则只展开这一个（收敛各调用点 `= if (== id) "" else id` 样板）。 */
    fun toggleDropdown(id: String) {
        openDropdown = if (openDropdown == id) "" else id
    }

    /** 置 dirty（任意表单字段变更时调用；与直接赋值 `dirty = true` 语义一致）。 */
    fun markDirty() {
        dirty = true
    }

    /**
     * 翻转以字符串存储的布尔字段并置 dirty。
     *
     * Kuikly Input/Switch 的绑定值为 String，布尔配置项（enabled/stream/use_itn…）在表单里
     * 以 "true"/"false" 字符串保存，保存时再 `toBooleanStrictOrNull()` 转回——见 [save]。
     * 接收 `KMutableProperty0`（如 `form.toggleStringBool(form::llmStream)`），对
     * [sm] / [cmd] / [wake] 子状态字段同样适用（`form.toggleStringBool(sm::soulEnabled)`）。
     */
    fun toggleStringBool(field: KMutableProperty0<String>) {
        field.set(if (field.get() == "true") "false" else "true")
        markDirty()
    }

    /** Top workflow and attached-node menu are projections of this single route. */
    private var configSelection by stateVar(ConfigSelection("vad", "vad"))
    var workspaceTab by stateVar(NodeWorkspaceTab.Configuration)
        private set
    var showingNodeStatus by stateVar(false)
        private set
    var selectedTestActionId by stateVar("")
        private set

    /**
     * 侧边栏「语音流程」与「全局配置」是两个独立菜单，但只有**一条**路由（[configSelection]）。
     * 这里分别记住两个菜单上次停留的位置：切换菜单时回到原处，而不是每次都被重置。
     * （不引入第二份 active 状态：位置记忆只是路由的历史值，不参与渲染判定。）
     */
    private var lastWorkflowSelection by stateVar(ConfigSelection("vad", "vad"))
    private var lastGlobalSection by stateVar(ConfigSections.global.firstOrNull()?.id.orEmpty())

    /** 当前路由是否落在「全局配置」菜单（= 选中的 section 没有流程节点）。 */
    val isGlobalSelection: Boolean get() = ConfigSections.isGlobal(configSection)

    /** 侧边栏进入「语音流程」：当前停在全局设置时，回到上次的流程位置。 */
    fun openWorkflowPage() {
        if (isGlobalSelection) selectConfigRoute(lastWorkflowSelection)
    }

    /** 侧边栏进入「全局配置」：当前停在流程节点时，回到上次的全局 section（可指定目标）。 */
    fun openGlobalPage(sectionId: String? = null) {
        val target = sectionId ?: lastGlobalSection.ifBlank { ConfigSections.global.firstOrNull()?.id.orEmpty() }
        if (target.isBlank()) return
        if (isGlobalSelection && configSection == target) return
        selectConfigRoute(ConfigSelection(target))
    }

    /**
     * 跨菜单跳转钩子：由壳页面注册，把侧边栏导航切到该 section 所属的菜单
     *（有流程节点 → 「语音流程」，否则 → 「全局配置」）。
     *
     * 为什么需要：卡片内的跳转（能力总览的健康修复、就绪度横幅…）只知道自己要打开哪个
     * section，并不知道当前在哪个菜单。没有这个钩子，「全局配置」页里点一个流程节点的链接，
     * 就会在全局菜单下渲染流程节点卡片——又回到两套语义混在一起的老问题。
     * 单页模式（`@Page("config")` 没有侧边栏）保持为 null，行为不变。
     */
    var onSectionRouted: ((String) -> Unit)? = null

    /** 全局配置左菜单选择：记下位置，切到语音流程再回来时仍在同一 section。 */
    fun selectGlobalSection(sectionId: String) {
        lastGlobalSection = sectionId
        openConfigSection(sectionId)
    }

    val workflowPosition: WorkflowPosition? get() = PipelineWorkflow.resolve(configSelection)
    val workspaceTitle: String get() = workflowPosition?.let {
        "${it.label} · ${if (it.isCore) "节点管理" else "扩展管理"}"
    } ?: "全局设置"

    /**
     * Rebuild only the right-hand pane on route, snapshot or local panel changes.
     * substringBefore('@') remains the config section; inputs still rebuild after async fill.
     */
    val configSectionList: MutableList<String> = stateListOf()
    init { configSectionList.add(configRenderKey()) }
    var configSection: String
        get() = configSelection.sectionId
        set(value) {
            val nodeId = if (ConfigSections.tabOf(value) == "voice") {
                PipelineFlow.primaryForSection(value)?.id
            } else null
            selectConfigRoute(ConfigSelection(value, nodeId))
        }

    val selectedFlowNodeId: String? get() = configSelection.nodeId

    fun selectFlowNode(nodeId: String) {
        PipelineFlow.selectionForNode(nodeId)?.let {
            selectConfigRoute(it.copy(workflowPositionId = PipelineWorkflow.forNode(nodeId)?.id))
        }
    }

    fun selectWorkflowPosition(positionId: String) {
        PipelineWorkflow.selectionForPosition(positionId)?.let { selectConfigRoute(it) }
    }

    /** Explicit repair/settings links open parameters, unlike top navigation which preserves the tab. */
    fun openConfigSection(sectionId: String) {
        configSection = sectionId
        showNodeParameters()
        selectWorkspaceTab(NodeWorkspaceTab.Configuration)
        // 菜单归属随 section 走（见 [onSectionRouted]）：跳到全局 section 就切到「全局配置」菜单
        onSectionRouted?.invoke(sectionId)
    }

    fun selectWorkspaceTab(tab: NodeWorkspaceTab) {
        if (workspaceTab == tab) return
        workspaceTab = tab
        openDropdown = ""
        notifyConfigReloaded()
    }

    fun showNodeParameters(nodeId: String? = selectedFlowNodeId) {
        if (nodeId != null) selectFlowNode(nodeId)
        showingNodeStatus = false
        notifyConfigReloaded()
    }

    fun showNodeStatus() {
        showingNodeStatus = true
        notifyConfigReloaded()
    }

    fun selectWorkspaceTest(nodeId: String, actionId: String) {
        selectFlowNode(nodeId)
        selectedTestActionId = actionId
        notifyConfigReloaded()
    }

    private fun selectConfigRoute(selection: ConfigSelection) {
        if (configSelection == selection) return
        configSelection = selection
        // 记住两个菜单各自的位置（全局 section 无流程节点 → 只有它属于「全局配置」）
        if (ConfigSections.isGlobal(selection.sectionId)) {
            lastGlobalSection = selection.sectionId
        } else {
            lastWorkflowSelection = selection
        }
        showingNodeStatus = false
        selectedTestActionId = ""
        openDropdown = ""
        nodeTests.invalidateRuns()
        notifyConfigReloaded()
    }

    private fun configRenderKey(): String =
        "${configSelection.sectionId}@$revision@${configSelection.nodeId.orEmpty()}@" +
            "${configSelection.workflowPositionId.orEmpty()}@$workspaceTab@$showingNodeStatus@$selectedTestActionId"

    /** 配置重载完成（fill 尾部调用）：按当前路由/版本重建内容，选择不被重载清除。 */
    internal fun notifyConfigReloaded() {
        val key = configRenderKey()
        if (configSectionList.isEmpty() || configSectionList[0] != key) {
            configSectionList.clear()
            configSectionList.add(key)
        }
    }

    /** 「高级选项」展开开关（P0-1 折叠高级字段；跨 section 共享，同一时刻只看一个 section）。 */
    var showAdvanced by stateVar(false)

    var testBusyLlm by stateVar(false)
    var testBusyTts by stateVar(false)
    var testBusyVad by stateVar(false)
    var testMsgLlm by stateVar("")
    var testMsgTts by stateVar("")
    var testMsgVad by stateVar("")
    var llmModelsBusy by stateVar(false)
    var llmModelsMsg by stateVar("")
    // Pair<id, label>，与 TTS 发音人下拉同构
    val llmModelOptions: MutableList<Pair<String, String>> = stateListOf()
    // 本地模型引擎（离线推理，凭据字段 = 模型路径等）。
    val ttsLocalEngines: MutableList<Pair<String, String>> = stateListOf()
    // 远程合成服务商（在线 API，凭据字段 = 各服务商密钥）。
    val ttsRemoteEngines: MutableList<Pair<String, String>> = stateListOf()
    // 合成方式两选项（local/remote）。
    val ttsModeOptions: MutableList<Pair<String, String>> = stateListOf()

    val nodeTests = NodeTestsState(scope)
    internal fun invalidateReadOnlyTests() {
        catalogueScopeEpoch++
        // The scope owner releases the old busy flag; stale callbacks must not release a
        // newer scope's request. No already-sent remote operation is claimed cancelled.
        llmModelsBusy = false
        testRequests.invalidate()
        nodeTests.invalidateScope()
    }
    fun refreshNodeTests(ctx: Pager) = nodeTests.refresh(ctx, serverBase, ctx.adminRequestToken(), editDeviceId)
    fun runNodeTest(ctx: Pager, descriptor: NodeTestDescriptor) = nodeTests.run(ctx, descriptor, serverBase, ctx.adminRequestToken(), editDeviceId)

    private val actions = ConfigFormActions(this, scope)
    private val testRequests = ConfigFormTests(this)

    init { initializeProviderCatalogue() }

    fun runInlineTest(ctx: Pager, kind: String) = testRequests.runInlineTest(ctx, kind)
    fun testXfyunCreds(ctx: Pager) = testRequests.testXfyunCreds(ctx)
    fun fetchLlmModels(ctx: Pager) = fetchProviderLlmModels(ctx)
    fun ttsEngineIsRemote(id: String): Boolean = ttsRemoteEngines.any { it.first == id }
    fun reloadXfyunVoiceOptions() = reloadProviderVoiceOptions()
    fun selectXfyunVoiceType(t: String) = selectProviderVoiceType(t)
    fun selectXfyunVoiceGroup(g: String) = selectProviderVoiceGroup(g)
    internal fun deriveVoiceGroupAndType() = deriveProviderVoiceGroupAndType()
    fun refreshVoices(ctx: Pager) = refreshProviderVoices(ctx)
    fun loadVoices(ctx: Pager, baseUrl: String = "") = loadProviderVoices(ctx, baseUrl)

    fun loadConfigMeta(ctx: Pager, baseUrl: String = "") = actions.loadConfigMeta(ctx, baseUrl)
    fun load(ctx: Pager, baseUrl: String = "") = actions.load(ctx, baseUrl)
    fun backToGlobal(ctx: Pager) = actions.backToGlobal(ctx)
    fun clearDeviceOverlay(ctx: Pager, onResult: ((String, String) -> Unit)? = null) =
        actions.clearDeviceOverlay(ctx, onResult)
    fun reload(ctx: Pager, baseUrl: String = "") = actions.reload(ctx, baseUrl)
    fun save(ctx: Pager, baseUrl: String = "", onResult: ((String, String) -> Unit)? = null) =
        actions.save(ctx, baseUrl, onResult)
    fun loadPresets(ctx: Pager, baseUrl: String = "") = actions.loadPresets(ctx, baseUrl)
    fun savePreset(ctx: Pager, baseUrl: String = "") = actions.savePreset(ctx, baseUrl)
    fun applyPreset(ctx: Pager, baseUrl: String = "", name: String) = actions.applyPreset(ctx, baseUrl, name)
    fun deletePreset(ctx: Pager, baseUrl: String = "", name: String) = actions.deletePreset(ctx, baseUrl, name)
}
