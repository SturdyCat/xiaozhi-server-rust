# NLU 构建与候选诊断

继承[网关指引](<../AGENTS.md>)。构建入口在[父模块](<../nlu_build.rs>)；进程在[training](<training.rs>)、版本清单在[artifact](<artifact.rs>)、Docker 路径适配在[mounts](<mounts.rs>)；[候选诊断](<../nlu_model.rs>)独立真实推理。

- 构建只能读 `engines.config_store.load()` 的最新 SQLite 保存快照，不能使用 `live_config()`、启动快照或读取失败后的旧配置回退；revision 与配置 API 使用同一个 `config_revision`。可选 `expected_revision` 不一致返回 409。
- 请求可为空以兼容旧管理台；新管理台应先保存技能，再提交 `{"expected_revision":"..."}`。未保存草稿不参与训练。
- 每次构建生成 UUID，工作目录 `<data>/nlu-train/<build_id>/`，产物目录 `<data>/models/nlu-builds/<build_id>/`；已存在目录拒绝复用，不覆盖现有模型。
- 数据根与文件管理保持一致，由保存配置 `memory.db_path` 的父目录派生。构建、候选测试都不写配置库、不刷新生产引擎。
- 训练脚本通过 `include_str!` 编译进服务，交给依赖镜像内 `python -c` 执行；不执行数据卷中的脚本，不执行用户输入代码。更新训练逻辑需发布新服务。依赖镜像只在缺失时拉取，然后 inspect 并固定 image id；日志不得打印整个脚本 argv。
- 预训练底座**唯一**：`minirbt-h288`。底座不是可选维度——`BuildRequest` 用 `deny_unknown_fields`，多传一个 `base_model` 直接 400；服务端只认 `base.rs` 的 `BASE_MODEL`。`train.py` 的 `BASES` 表与 Dockerfile 的 `ENV MODEL_NAME=` 必须与它**逐字段一致**（`tests.rs` 用 `include_str!` 解析源码锁定 hub/lr/epochs），否则日志说一套、实际训另一套。
- 训练输入共用 `plugins::skill::effective_catalog(command, nlu)`：保存态技能与启用的 command 词表（`end_session`）统一成唯一目录。默认技能是配置种子，显式清空不隐藏补回；新训练标签就是目录名，新构建清空外部旧模型的 `bert_label_map` 为同名映射。
- 成功只表示 Docker 退出成功、三件产物齐全、标签顺序严格等于有效目录顺序加 `other`、文件哈希已记录；**不代表模型准确率合格**。手动构建不自动启用；首启自举按 `NLU_BOOTSTRAP` 决定（默认 `adopt`：冒烟自检通过才自动采用；`build-only` 只训不启用；`off` 关闭）。冒烟只证明"没坏"，不构成效果验收，文案禁止出现"验收通过/准确率/可用"。新 manifest 记录 `catalog_source=effective_command_nlu_v1`；旧 manifest 缺该字段可正常读取，始终使用原快照与原 labels，不伪造新增 `end_session` 标签。
- `manifest.json` 保存 build id、来源 revision、训练输入哈希、依赖镜像 tag/id、可信脚本 SHA-256、标签清单、模型/分词器/标签文件身份和 NLU 推理快照。公开 artifact 不返回整份 NLU 快照。
- 状态与日志 job 是进程态，重启后回 idle；成功清单持久化，已知 build id 的候选仍可测试。尚未实现历史版本列表/自动清理。
- 候选测试只接受 `build_id/text/expected_intent?`，不接受任意模型路径或配置补丁；只查受管版本目录，拒绝非规范 UUID、目录或文件符号链接、越界路径、缺清单、哈希变化和标签变化。
- 候选测试在有界 `spawn_blocking` 任务中构造真实 `BertClassify`，使用构建快照的阈值、映射、线程、截断参数，仅在局部对象设 `bert_enabled=true`；绝不持久化或发布。加载与推理超时后，许可仍由后台任务持有，避免继续堆积模型。
- `accepted` 仅是意图分类阈值决策，不表示槽位验证、技能可执行或设备执行。`other` 无条件拒识；低分、映射未知技能拒识。`expected_match` 按接受决策与人工预期计算。
- 采用候选仍走显式配置 POST，使用 artifact 的 `expected_config`；它包含版本路径、阈值、映射与推理参数，避免“测试一个配置、启用另一个配置”。技能内容是否在测试后又被改动由调用者通过 revision/来源清单检查，不做自动替换。
- 构建、状态、日志和候选测试各自检查管理鉴权；真实训练与真实候选推理另检查 `dev_mode` 并返回 503，不能依赖调用者恰好挂了某个中间件。
- 本地回归只覆盖保存态快照、版本/完整性与路径边界、鉴权/dev/revision、分类拒识与无配置副作用；假字节模型从不进入真实推理，不能将测试称为真实模型验收。真实环境需用户发布新服务后执行控制面板闭环。

## 已知边界

- **配置保存没有真正的 CAS**：`save_patch` 是 `read → compare → persist`，`compare` 与 `persist` 之间的亚秒窗口里若有人写入，会被本次保存静默覆盖（last-write-wins）。彻底修需要 store 层支持 put-if-revision。首启自举的 `stale` 只挡得住"训练期间配置被改"，挡不住这个窗口。
- **首启自举的两类失败方向不能反**：`RevisionConflict`（构建依据的配置已过期）→ 立即 `stale` 终态，**不重试**（用同一个 `expected_revision` 重试永远不可能成功）；`Io`/`Persist`（SQLite busy、磁盘抖动）→ 才重试 200/400/800ms。自举读配置必须走 `save_patch_strict`（读失败即失败），不能用 `current_config()` 的内存回退——否则一次读库抖动会被误判成"配置被别处改过"而永久 `stale`，理由还是假的。
- **部署是硬顺序：先推训练镜像，再发服务**。服务内嵌的脚本会引用镜像里烘好的权重；镜像没到位时构建必然失败（`image_missing` / `weights_unavailable`），且 `giving_up` 之后必须人工删 `<data>/nlu-bootstrap/state.json` 才会重新自举——不要指望"失败自愈"。来不及推镜像就先设 `NLU_BOOTSTRAP=off`。
- **冒烟用例全部取自训练语料**（模板展开句 + `OTHER_CORPUS`），它证明的是"训练/导出/量化这条流水线没坏"，不构成效果验收，也不能当准确率对外说。
- **首启自举会在启动 30s 后自动训练、并可能自动启用模型**。`bert_enabled=false` + 空路径同时是"从未配置过"和"用户主动关掉"的形态，配置上不可区分，所以不猜意图：只把后果写进启动日志，要关闭请设 `NLU_BOOTSTRAP=off`。
- 模型文件和 manifest 处于管理员可写数据卷。哈希检测意外变化，不是抵抗已获管理员文件写权限者同时重写清单的签名系统，也不保证与并发外部文件修改之间的内核级不可变快照。
- Docker pull/run 命令最长等待 3600 秒；inspect 等探测未另设独立 deadline。超时会终止 CLI，但宿主容器可能需要管理员后续清理；资源上限仍由 Docker 限制。未提供取消按钮。
- 第一次缺失镜像、HF 缓存或不兼容的 ML 依赖会使构建失败，保留旧生产模型；本模块不通过运行期 pip 自动修补环境。
