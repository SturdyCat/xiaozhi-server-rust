# minirbt-h288 构建修复与同输入对照验证（2026-11-02）

## 范围与结论

- 目标：验证 `minirbt-h288`（下称 h288）NLU 模型——构建、微调、真实推理评测，并与此前 `rbt3` 模型对比处理速度与得分。
- NAS `192.168.99.249` 已更新到支持 `base_model` 的最新版：guard 探测确认（过期 revision → 409；非法 base → 400 `invalid_base_model`）。
- **首次受管构建秒败（exit 125）**：根因是服务端 `env_args()` 只生成裸 `KEY=VALUE`、缺 `-e` 旗标，`docker run` 把 `MODEL_NAME=minirbt-h288` 解析成镜像名（`library/MODEL_NAME=... must be lowercase`）。该缺陷使新服务上**任何底座**的构建都会同样失败。已在源码修复并通过回归；**NAS 复验需重新发布服务后执行**。
- 修复期间用失败构建的**同一份训练输入字节**（SHA-256 `bc91a77a3ca7042d07c1617eb1b8be0c45efc6dfd45ce13cef3ff1044e823957`，3196 字节，含当前保存态 `end_session` 词表，语料 835 条）在本机完成 h288 与 rbt3 双臂微调，并在同机同线程配置下用冻结 54 例真实 ONNX 推理对照。
- **本机对照结论**：h288 推理中位 **1.06 ms** vs rbt3 **2.46 ms**（约 2.3×），产物 **12.65 MB** vs **38.72 MB**（约 1/3）；正例接受 h288 17/18 vs rbt3 15/18，泛化 top-1 12/12 vs 9/12；但 h288 负例/歧义误接受 15/24 劣于 rbt3 的 11/24。**两臂都未达到"负例不误接受"的安全启用标准**，维持上一轮结论：生产 BERT 保持关闭，未采用任何候选。
- 生产配置本轮零改动：保存态 revision 始终 `b47a012acae9414e`，`bert_enabled=false`、阈值 `0.75`、生产模型路径不变；失败的构建 `7a30446a` 只留下空工作/产物目录（UUID 目录，不覆盖旧模型），符合"构建不改生产"的设计。

## 源码修复（待发布）

- [base.rs](<../server/src/gateway/ws/nlu_build/base.rs>)：`env_args()` 改为 `-e KEY=VALUE` 成对输出；注释记录 2026-11-02 实测失败形态。
- [tests.rs](<../server/src/gateway/ws/nlu_build/tests.rs>)：契约测试改为锁定**含 `-e` 的完整 docker 参数**，并加结构断言（每个 env 值必须紧跟 `-e`）——旧测试只锁值，挡不住这次回归。
- [train.py](<../tools/nlu-trainer/train.py>)：checkpoint/fp32 中间目录改由 `TRAIN_CKPT`/`TRAIN_FP32` 覆盖（容器内缺省 `/work/*` 不变），standalone 无需根目录 `/work`。
- 验证：`cargo test --locked` 全量 **1143 通过 / 0 失败 / 6 忽略**（其中 `nlu` 过滤 90 项含修复契约）；训练器标准库回归 **38/38**。三个 unused warning 均为既有代码，未动。
- 发布注意：只需重建并发布服务端（训练镜像无变化）；发布后用 `.dev/nlu-validation-20261102/h288_validate.py` 的 `probe → start → watch → download → evaluate` 重跑受管构建与 NAS 真机推理。

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

- 本机时延**不代表 NAS**：上一轮同端点记录的 rbt3 NAS 推理为 23–51 ms（中位 36 ms），NAS 硬件明显更慢；h288 的 NAS 真实时延须在重新发布后用同一候选端点实测，不能由本机倍数外推。
- 本机训练用时跨 torch 版本（h288 臂固定 torch 2.6 后重训），不作训练效率结论。

## 与上一轮 rbt3 模型（2026-11-01 NAS，831 条语料、无 end_session）的关系

- 那一版 rbt3 的 54 例成绩（正例接受 22/30、不行动未通过 10/24）用的是**旧训练输入**，与本轮 835 条（含 `end_session`）不同，分数不可直接横比；本轮受控对照（同输入双臂）才是公平口径。
- 速度的权威对比同样要以 NAS 复验为准：同端点、同硬件、同方法（`/api/nlu/model/test` 的 `infer_ms`）。

## 边界

- 54 例为手写固定集，非统计代表性测试；本轮只做意图分类与保守拒识验证，不含槽位抽取、任务创建、设备执行或端到端语音链路。
- 两臂负例误接受都远超可接受范围，**不建议在修复负样本语料前启用生产 BERT**；本轮未调整阈值、未改人工标签、未为刷分反复调参。
- 失败构建 `7a30446a` 的工作/产物目录留在 NAS 数据卷（空目录 + input.json），无副作用；重试会生成新 UUID。
- 本机对照在 macOS CPU（onnxruntime 1.31.1）完成；本地脚本修复 ≠ 线上已部署，NAS 结论以复验轮为准。

## 证据（`.dev/nlu-validation-20261102/`）

- 构建失败与诊断：`build-terminal.json`、`build-log-frames.json`、`server-log-backlog.json`（docker 报错原文）
- 同输入证据：`training-input.json`（SHA-256 与失败构建 `input_sha256` 一致）
- 训练日志：`train-h288.log`、`train-rbt3.log`（corpus_stats / split_stats / base= / eval metrics / smoke）
- 评测：`local-comparison.json`、`controlled-comparison.json`、`local-h288-output-evaluation.json`、`local-rbt3-output-evaluation.json`
- 操作脚本：`h288_validate.py`（probe/start/watch/download/evaluate/report 七阶段，复用 2026-11-01 受审 API 路径与冻结样例）、`local_eval.py`
- 配置基线：`before-config.json`、`guard-probes.json`（revision `b47a012acae9414e` 基线与 409/400 guard 证据）
