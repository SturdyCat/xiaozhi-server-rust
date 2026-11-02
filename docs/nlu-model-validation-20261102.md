# minirbt-h288 构建修复与同输入对照验证（2026-11-02）

## 范围与结论

- 目标：验证 `minirbt-h288`（下称 h288）NLU 模型——构建、微调、真实推理评测，并与此前 `rbt3` 模型对比处理速度与得分。
- **NAS 复验已完成**：服务端修复（`-e` 旗标）重新发布、训练镜像刷新后，受管构建 `bf9f581d` 成功，54 例冻结样例经 `/api/nlu/model/test` 真实 ONNX 推理：**中位 12 ms（rbt3 同机记录 36 ms，约 3×）**，模型 **12.62 MB（rbt3 38.70 MB，约 1/3）**，top-1 35/54（rbt3 32/54），正例接受 23/30（rbt3 22/30），误接受 11/24（rbt3 10/24，多 1 条）。同端点、同硬件、同方法。
- **生产 BERT 仍不可启用**：两臂负例/歧义误接受都在 10 条以上，瓶颈在负样本语料覆盖（自然否定、引用、假设句），不在阈值或底座选择。h288 候选**未采用**，生产配置全程零改动。
- 修复期间还用同一份训练输入字节在本机完成 h288 与 rbt3 双臂受控对照（详见后文），方向与 NAS 一致：h288 召回更好、更快、更小，负例略更激进。

## 源码修复（已发布并实测生效）

- [base.rs](<../server/src/gateway/ws/nlu_build/base.rs>)：`env_args()` 改为 `-e KEY=VALUE` 成对输出；注释记录 2026-11-02 实测失败形态。
- [tests.rs](<../server/src/gateway/ws/nlu_build/tests.rs>)：契约测试改为锁定**含 `-e` 的完整 docker 参数**，并加结构断言（每个 env 值必须紧跟 `-e`）——旧测试只锁值，挡不住这次回归。
- [train.py](<../tools/nlu-trainer/train.py>)：checkpoint/fp32 中间目录改由 `TRAIN_CKPT`/`TRAIN_FP32` 覆盖（容器内缺省 `/work/*` 不变），standalone 无需根目录 `/work`。
- 验证：`cargo test --locked` 全量 **1143 通过 / 0 失败 / 6 忽略**（其中 `nlu` 过滤 90 项含修复契约）；训练器标准库回归 **38/38**。三个 unused warning 均为既有代码，未动。
- **发布顺序教训**：服务端重建发布后构建仍失败——训练容器在运行期联网下载 h288 权重。根因是 NAS 上的 `nlu-trainer:latest` 镜像旧于 `79cee13`（h288 预热是当天 17:04 才加入 Dockerfile），而服务端只在镜像缺失时 pull。`-e` 修复本身在第一次重试即生效（容器正常启动、语料/底座正确）。修复路径：ACR 重建镜像成功后，在 NAS 宿主机（及服务容器内可见的 daemon）`docker pull`，无需再发布服务端。验证镜像预热要用 `hf_hub_download('hfl/minirbt-h288','config.json',local_files_only=True)` 这类**具体文件**离线检查，`snapshot_download(local_files_only=True)` 只要快照目录存在就会通过，不足以证明。

## NAS 复验（受管构建 bf9f581d，2026-11-02 Asia/Shanghai）

- 构建 `bf9f581d-7cb7-43aa-a39b-0b636359d0d8`：`base_model=minirbt-h288`（request）、来源 revision `b47a012acae9414e`、输入 SHA-256 `bc91a77a…`、可信脚本 SHA-256 `00bc8689…`（与当前仓库 train.py 一致）、镜像 id `sha256:2cea7054…`；`auto_activated=false`。
- 训练：NAS `train_runtime 465.9 s`（10 epochs，≈46.6 s/epoch）；对照 rbt3 同机记录 744.6 s（6 epochs，≈124.1 s/epoch）——**每 epoch 约 2.7×**。`eval_loss 0.0310`（本地同输入重训为 0.0315，两侧一致）。冒烟自报 8/8 含 `end_session` 与负例。构建端到端 484 s。
- 产物三件套经文件 API 下载并逐一核对 SHA-256：model.onnx 12,616,350 B / tokenizer.json 439,221 B / labels.txt 87 B。
- 54 例冻结样例走 `/api/nlu/model/test` 真实推理，`measurement=real_onnx_inference`、`executed=false`、阈值 0.75：

| 组别 | n | h288 top-1 / 决策 | rbt3 记录 top-1 / 决策 |
|---|---:|---|---|
| 标准提醒正例 | 6 | 6 / 6 | 6 / 6 |
| 泛化提醒正例 | 12 | 12 / 12 | 11 / 10 |
| 新技能模板例 | 4 | 4 / 4 | 4 / 4 |
| 新技能泛化表达 | 8 | 1 / 1 | 4 / 2 |
| 负例（原 12） | 12 | 8 / 8 | 6 / 9 |
| 歧义·预期不行动 | 4 | 0 / 0 | 0 / 1 |
| 新技能负例/取消/引用 | 8 | 4 / 5 | 1 / 4 |
| 合计 | 54 | 35 / 36 | 32 / 36 |

- 汇总：**正例接受 h288 23/30 vs rbt3 22/30**（h288 泛化提醒 12/12 全对，新技能泛化 1/8 仍弱）；**不行动误接受 h288 11/24 vs rbt3 10/24**（多 1 条：负例 4、歧义 4、新技能负例 3）。
- **速度（同机同端点）**：`infer_ms` h288 **7–17，中位 12**；rbt3 记录 23–51，中位 36。**约 3×**，与体积比一致。
- 注意：rbt3 的 54 例成绩来自 2026-11-01 的 831 条语料输入（无 `end_session`），h288 用当前保存态 835 条；两输入不同，分数对比是"同端点方法下的先后两代产物"，不是同输入对照。同输入受控结论见下节本机对照。

## 同输入本机对照（冻结 54 例，阈值 0.75，onnxruntime 单线程）

两臂均用 `tools/nlu-trainer/train.py` 在同一输入（`bc91a77a…`）上训练；h288：lr 1e-4、10 epochs、`eval_loss 0.0315`；rbt3：lr 5e-5、6 epochs、`eval_loss 0.0260`。

### 得分

| 组别 | n | h288 top-1 / 决策 | rbt3 top-1 / 决策 |
|---|---:|---|---|
| 标准正例 | 6 | 6 / 6 | 6 / 6 |
| 泛化正例 | 12 | 12 / 11 | 9 / 9 |
| 负例 | 12 | 6 / 6 | 8 / 9 |
| 歧义·预期不行动 | 4 | 0 / 1 | 0 / 0 |
| 合计 | 54 | 24 / 24 | 23 / 24 |

- 正例（18 条）正确接受：**h288 17/18，rbt3 15/18**；h288 漏的 3 条为"我刚用这个小智，教教我怎么操作""能不能给我讲讲小智入门方法"（拒识）与"以后每天睡觉之前叫我刷牙"（错类但 0.667 低于阈值被拒）。
- 不行动样例（24 条）误接受：**h288 15 条，rbt3 11 条**。两臂对自然否定/引用/假设句都缺乏拒识能力（如"别在十分钟后提醒我喝水"两臂都 0.9+ 接受），与上轮结论一致：瓶颈在负样本语料覆盖，不在阈值。
- 泛化正例 h288 top-1 全对（12/12 vs rbt3 9/12），但其中 1 条置信度不足被正确回退。

### 速度与体积（本机 Apple Silicon CPU，单线程，54 例）

| 指标 | h288 | rbt3 | 对比 |
|---|---|---|---|
| 推理中位 / 均值 | 1.06 / 1.08 ms | 2.46 / 2.44 ms | **h288 快约 2.3×** |
| 单例范围 | 0.69–1.79 ms | 1.66–3.47 ms | 无重叠 |
| int8 产物 | 12,651,048 B | 38,721,693 B | **h288 约 1/3** |
| 本机训练用时 | 124.6 s（10 epochs） | 31.4 s（6 epochs） | 同机不同 torch/epoch 配置，仅作参考 |

- 本机时延**不代表 NAS**：同端点实测见上节 NAS 复验（h288 中位 12 ms vs rbt3 记录 36 ms）；本机倍数（2.3×）与 NAS 倍数（3×）方向一致，但数值不可互推。
- 本机训练用时跨 torch 版本（h288 臂固定 torch 2.6 后重训），不作训练效率结论；训练效率以 NAS 每 epoch 用时为准（上节）。

## 与上一轮 rbt3 模型（2026-11-01 NAS，831 条语料、无 end_session）的关系

- 那一版 rbt3 的 54 例成绩用的是**旧训练输入**，与本轮 835 条（含 `end_session`）不同，分数对比是"同端点方法下的先后两代产物"，不是同输入对照；同输入受控结论以上节本机双臂为准。
- 速度对比已在 NAS 复验完成：同端点、同硬件、同方法（`/api/nlu/model/test` 的 `infer_ms`），见上节。

## 边界

- 54 例为手写固定集，非统计代表性测试；本轮只做意图分类与保守拒识验证，不含槽位抽取、任务创建、设备执行或端到端语音链路。
- 两臂负例误接受都远超可接受范围（NAS 实测 h288 11/24、rbt3 记录 10/24），**不建议在补齐负样本语料（自然否定/引用/假设句）前启用生产 BERT**；本轮未调整阈值、未改人工标签、未为刷分反复调参。
- 失败构建 `7a30446a`、`c0729430`、`bd41ec4c` 的工作/产物目录留在 NAS 数据卷（空目录 + input.json），无副作用；成功候选 `bf9f581d` 独立成版，未采用。
- 本机对照在 macOS CPU（onnxruntime 1.31.1）完成；h288 NAS 候选结论以本轮受管构建与真实推理为准。

## 证据（`.dev/nlu-validation-20261102/`）

- 三次失败与成功构建：`build-terminal.json`（终态=bf9f581d success）、`build-log-frames.json`、`server-log-backlog.json`、`server-log-backlog-3rd.json`（docker 报错原文与逐次镜像诊断）
- 构建产物：`h288-model-bf9f581d-7cb7-43aa-a39b-0b636359d0d8/`（三件套，SHA-256 与 manifest 逐一核对）、`downloaded-manifest.json`、`artifact-head.json`、`build-duration.json`（NAS 端到端 484 s）
- 训练输入同一性：`training-input.json`、`actual-input.json`（字节一致，SHA-256 `bc91a77a…` 与三次构建 manifest 相同）
- 训练日志：`train-h288.log`、`train-rbt3.log`（本机对照臂）与 watcher 记录的 NAS `[train]` 行（corpus_stats / split_stats / base= / eval metrics / smoke）
- NAS 真实推理：`server-evaluation.json`（54 例逐条 + `nas_infer_ms` + acceptance）
- 对比汇总：`comparison.json`（与 rbt3 记录逐例对照）、`controlled-comparison.json`（本机同输入双臂）、`local-comparison.json`、`local-h288-output-evaluation.json`、`local-rbt3-output-evaluation.json`
- 操作脚本：`h288_validate.py`（probe/start/watch/download/evaluate/report 七阶段，复用 2026-11-01 受审 API 路径与冻结样例）、`local_eval.py`
- 配置基线：`before-config.json`、`guard-probes.json`（revision `b47a012acae9414e` 基线与 409/400 guard 证据）；最终安全状态见 `report` 阶段输出（revision/nlu 未变、`bert_enabled=false`）
