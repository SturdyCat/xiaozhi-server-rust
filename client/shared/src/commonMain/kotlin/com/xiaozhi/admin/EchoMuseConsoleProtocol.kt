package com.xiaozhi.admin

/*
 * 控制台的**跨端契约**：帧协议字面量（设计 §8.1）与用户可见文案（§8.3）。
 *
 * 为什么从 EchoMuseConsoleState 单独成文件：这两组常量与状态机无关，却被三处同时用——
 * 状态机解析帧、EchoMuseConsoleTab 渲染提示、以及跨端"改名即协议破坏"的对齐要求
 * （服务端 console_frames.rs 与 console_tests.rs::frame_types_and_codes_match_shared_contract
 * 逐字比对本文件的字符串）。留在状态文件里会让状态机顶过 600 行硬上限，而这两组常量
 * 恰恰是**不能随状态机一起被压缩/删减**的部分。
 *
 * 任何一侧改名都是协议破坏：type / error.code / closed.reason / notice.code / ready.mode
 * 五个集合必须与服务端 serde 派生结果逐字一致；用户文案必须与 console_frames.rs 的
 * COPY_* 逐字一致（同一个失败从哪一侧撞上都读到同一句话）。
 */

// ============================================================
// §8.1 字面量（客户端唯一出处，与 `server/.../console_frames.rs` 逐字一致）
// ============================================================

/**
 * 帧协议字面量。服务端由 serde 派生（`rename_all = "kebab-case"`）与
 * `CloseReason::code()` 决定，客户端只认这些字符串——**两侧任何一侧改名都是协议破坏**，
 * 服务端 `console_tests.rs::frame_types_and_codes_match_shared_contract` 与
 * 本文件的字面量必须同时改。
 */
internal object ConsoleWire {
    // server → browser
    const val READY = "ready"
    const val OUTPUT = "output"
    const val NOTICE = "notice"
    const val ERROR = "error"
    const val CLOSED = "closed"
    const val PONG = "pong"

    // browser → server
    const val INPUT = "input"
    const val CTRL_C = "ctrl-c"
    const val PING = "ping"

    /** 心跳帧原文（`{"type":"ping"}` → `{"type":"pong"}`）。 */
    const val PING_FRAME = "{\"type\":\"ping\"}"

    /** 中断帧原文（行模式无 tty，`ctrl-c` 是唯一的"信号"通道）。 */
    const val CTRL_C_FRAME = "{\"type\":\"ctrl-c\"}"

    // error.code
    const val CODE_BUSY = "busy"
    const val CODE_INPUT_TOO_LONG = "input-too-long"
    const val CODE_RATE_LIMITED = "rate-limited"
    const val CODE_DEVICE_OFFLINE = "device-offline"
    const val CODE_INTERNAL = "internal"

    // closed.reason
    const val REASON_IDLE = "idle-timeout"
    const val REASON_LIMIT = "session-limit"
    const val REASON_OFFLINE = "device-offline"
    const val REASON_SUPERSEDED = "superseded"
    const val REASON_SHELL_LOST = "device-shell-lost"
    const val REASON_REPLACED = "replaced"
    const val REASON_SERVER_SHUTDOWN = "server-shutdown"
    const val REASON_INTERNAL = "internal"
}

/**
 * 用户可见文案（§8.3 定稿，**逐字**；禁止在渲染层另写一份）。
 *
 * 可见性是 `internal` 而非设计稿建议的 `private`：控制台视图（`EchoMuseConsoleTab.kt`）
 * 必须复用同一条 `insecure` 警告与审计提示，各写一份就会漂移——**文案唯一出处**这个
 * 约束比"只在本文件可见"更重要。
 *
 * [INPUT_TOO_LONG] / [RATE_LIMITED] 是客户端**前置拦截**时用的兜底文案：正常路径下
 * 这两句也在服务端 `error.error` 里发回来（`console_frames.rs` 的 `COPY_*`），
 * 两处措辞一致，用户从哪个方向撞上都读到同一句话。
 */
internal object ConsoleCopy {
    const val CONNECTING = "正在与设备建立会话…"
    const val CONNECTED = "已连接 · 终端"
    const val OFFLINE = "设备离线——控制台不可用"
    const val BUSY = "设备已被另一个操作占用（系统信息/日志/资产下发），请稍后重试"
    const val IDLE_TIMEOUT = "会话因 10 分钟无输入已断开（输出已保留）"
    const val SESSION_LIMIT = "会话已达 2 小时上限，已断开（输出已保留）"
    const val DEVICE_LOST = "设备已断开——控制台关闭"
    const val SUPERSEDED = "设备已重连，旧会话已失效"
    const val SHELL_LOST = "设备端 shell 已退出（可能被重启命令中断）"

    /**
     * 会话被别的操作接管（`closed.reason=replaced`，服务端防御性分支）。
     *
     * 措辞刻意点明「**输出已保留**」：会话没了但缓冲还在，用户不必再连一次只为看一眼报错。
     */
    const val REPLACED = "会话被其他操作接管，已断开（输出已保留）"

    /**
     * `internal` 类失败的统一前缀，**必须与 `console_frames.rs::COPY_INTERNAL` 逐字一致**。
     *
     * 服务端 `error{code:"internal"}` 的 `error` 字段是 `COPY_INTERNAL + detail`（已带前缀），
     * 而 `closed{reason:"internal"}` 的 `detail` 是**裸原因文本、无前缀**——所以只有
     * `closed` 路径需要本端自己拼前缀，`error` 路径再拼一次就会变成「控制台内部错误：控制台
     * 内部错误：…」的双前缀。两条路径的差异见 [ConsoleHint]。
     */
    const val INTERNAL_PREFIX = "控制台内部错误："

    const val UNSUPPORTED = "当前平台暂不支持控制台（Android 客户端未接入 WebSocket）"
    const val RECONNECT_LOST = "连接中断（服务端已退出或网络中断）"

    /** `insecure` 琥珀警告条（PRD §5.6）。 */
    const val INSECURE_WARN = "设备未设置 token，同网段任何人可连入——root 命令通道不建议在此配置下使用"

    /** 缓冲截断提示（[EchoMuseConsoleText.TRUNCATED_NOTICE] 同值，这里给它一个协议内名字）。 */
    const val TRUNCATED = EchoMuseConsoleText.TRUNCATED_NOTICE

    /** 会话输出封顶（服务端也会发同文案的 `notice`）。 */
    const val CAPPED = "会话输出已达 16 MiB 上限，后续输出不再显示（命令仍可执行）"

    const val INPUT_TOO_LONG = "单行命令过长（上限 4096 字节），未发送"
    const val RATE_LIMITED = "输入过快，已丢弃（每次最多 10 条/秒）"
}

// ============================================================
// 帧 → 用户文案（纯映射，无状态、无 IO）
// ============================================================

/**
 * `closed{reason, detail}` → 状态行文案的**纯映射**。
 *
 * 为什么单独成对象而不是留在 `EchoMuseConsoleState` 的私有方法里：这段是"协议字段 →
 * 中文一句话"的查表，没有任何状态与副作用；但 `EchoMuseConsoleState` 的构造要 Kuikly 的
 * `Pager.observable`，而 `client/adminCoreTests` 刻意**不引 Kuikly**（只编译纯 Kotlin 源码），
 * 混在状态机里就只能靠"造假 Pager + 假 WS 回调"才能测。搬出来后这层断言是 `assertEquals`，
 * 状态机那边只剩"取字段 → 查表 → 写 observable"三行，状态机本身没被拆散。
 *
 * ⚠️ `reason=internal` **必须**把 `detail` 拼上：它是这条失败的唯一信息源
 * （`console.rs` 的 `DETAIL_DIAL_TIMEOUT`「设备未在 15 秒内回拨 shell 通道」、
 * `DETAIL_DIAL_CLOSED`「设备回拨通道已关闭（占槽在回拨前被撤销）」）。
 * 丢掉它 = 把所有内部错误都说成「服务端已退出或网络中断」，会把排查方向直接带偏
 * （用户会去查网络，实际是设备没回拨或占槽被撤销）。
 */
internal object ConsoleHint {
    /**
     * `closed` 帧 → 收尾文案。输出**保留**（用户还得看断开前的输出）。
     *
     * `detail` 在协议里是**裸原因文本、无前缀**（与 `error.error` 不同，后者由服务端拼好
     * `COPY_INTERNAL`）；所以只有 `internal` 这一支要本端补 [ConsoleCopy.INTERNAL_PREFIX]。
     */
    fun closed(reason: String, detail: String): String = when (reason) {
        ConsoleWire.REASON_IDLE -> ConsoleCopy.IDLE_TIMEOUT
        ConsoleWire.REASON_LIMIT -> ConsoleCopy.SESSION_LIMIT
        ConsoleWire.REASON_OFFLINE -> ConsoleCopy.DEVICE_LOST
        ConsoleWire.REASON_SUPERSEDED -> ConsoleCopy.SUPERSEDED
        ConsoleWire.REASON_SHELL_LOST -> ConsoleCopy.SHELL_LOST
        // 防御性分支（槽被非预期接管）：输出保留，用户不必为了看一眼报错再连一次
        ConsoleWire.REASON_REPLACED -> ConsoleCopy.REPLACED
        // 服务端每条 internal 都带 detail；真为空只可能是协议异常，此时宁可退成通用
        // "连接中断"，也不显示一句光有前缀的「控制台内部错误：」
        ConsoleWire.REASON_INTERNAL ->
            if (detail.isEmpty()) ConsoleCopy.RECONNECT_LOST else ConsoleCopy.INTERNAL_PREFIX + detail
        // 保留码：服务端一期不发（无优雅退出，设计 §0.3），收到当普通中断处理
        ConsoleWire.REASON_SERVER_SHUTDOWN -> ConsoleCopy.RECONNECT_LOST
        else -> ConsoleCopy.RECONNECT_LOST
    }
}
