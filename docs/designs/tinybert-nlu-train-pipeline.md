# TinyBERT NLU 训练流水线方案（设计稿，未实施）

- 状态：**方案记录，尚未实施**。当前已交付的是静态 TinyBERT 推理链路（`[nlu]` bert_* 配置、`server/src/plugins/nlu/bert.rs`、三级级联规则→TinyBERT→LLM、`nlu_test` 协议与测试台 NLU tab）；本文记录"配置界面一键重训模型"的自动化方案与待定决策。
- 背景：意图标签集因部署而异，TinyBERT 模型必须按技能定义微调 + 导出 ONNX；用户期望"在配置管理界面添加/管理 skill 后触发生成模型"，训练在远程服务器上通过 Docker 完成。

## 一、目标链路

```
管理台 [nlu] 卡片：编辑 skills/intents → 点「重新生成模型」
   ↓ WS: NluTrainStart（复用 bench 协议模式，同 nlu_test/nlu_test_result 先例）
服务端编排（Rust，零训练依赖）：
  ① 导出训练规格：解析当前 [nlu].intents/slots + 槽位值表
     → /data/nlu-training/spec.json（种子句、负样本、标签表）
  ② （可选）用已配云 LLM 把种子句改写成口语变体，每意图凑 N 句
  ③ docker run --rm -v /data/nlu-training:/work 专用训练镜像（依赖在镜像构建期装好，见 tools/nlu-trainer/Dockerfile，阿里云 ACR 构建发布）
     容器内：读 spec.json → 微调 TinyBERT4 → optimum 导出 → int8 量化
     产物写回 /work/out/：model.onnx + vocab.txt（或 tokenizer.json）+ labels.txt
  ④ 服务端验收：三件套齐全、labels 行序与 bert_label_map 对齐、试推理一句
  ⑤ 原子替换到 [nlu].bert_model 等路径 → 配置库记签名 → 热生效
     （Engines bert 缓存按签名重建，失败缓存 None 只 warn，会话不断）
   ↓ WS: NluTrainResult { state, elapsed_ms, eval 指标, model_bytes, error? }
管理台：按钮态 + 训练中进度 + 完成/失败提示
```

## 二、关键设计点

| 项 | 结论 |
|---|---|
| 训练环境 | 专用训练镜像（`tools/nlu-trainer/Dockerfile`，基于 `python:3.11-slim`）：Python 依赖与训练脚本在镜像构建期全部装好，服务端只 `docker pull` + `docker run`，无运行期 pip 引导。TinyBERT4 仅 14.5M 参数，GPU 非必需，CPU 十分钟内量级 |
| 服务器前置条件 | 宿主机装有 Docker；server 容器需挂载 `/var/run/docker.sock`（或宿主 docker CLI）并共享 `/data` 卷 |
| CPU 争抢 | 容器内 `torch.set_num_threads` 限核 + nice 低优先级；训练是一次性事件，不进常驻推理预算（`workflow/pipeline/inference.rs` 闸门不管训练进程） |
| Rust 侵入面 | 仅四块：spec 导出器、docker 子进程封装（超时 + 取消 + 日志尾随）、三件套验收器、`NluTrainStart/NluTrainResult` 两条 WS 消息 + 注册表字段 |
| 失败边界 | 训练失败或验收不过 → 旧模型继续使用；NLU 回退规则 + 启发式 + LLM 级联，会话永不中断 |
| 模型产出 | int8 量化后约 15~20MB 三件套，路径即 `[nlu].bert_model/bert_vocab/bert_labels` 指向处；`labels.txt` 行序 = 标签序，`bert_label_map` 映射回技能名 |

## 三、语料生成

1. **枚举**：把 `[nlu].intents` 的 `templates`（含 `{slot}`）按 `slots` 值表笛卡尔积代入得到种子句；同时生成交叉歧义负样本（如"把客厅的灯关掉"不得标成 `light_on`）。
2. **泛化（待定）**：用服务端已配云 LLM 批量改写种子句为口语变体（"帮我把客厅那个灯开一下呗"），每意图 100~300 句；LLM 失败降级为纯模板展开。
3. **对齐**：labels 与技能名经 `bert_label_map` 映射；未覆盖输出交给现有三级级联兜底。

## 四、待定决策（实施前需确认）

- **语料泛化方式**：LLM 改写（识别率更好，依赖云 LLM）vs 仅模板展开（零成本，语料生硬，需人工多写模板）。
- **实施范围**：全链路一次闭环（spec 导出 + docker 封装 + 验收热更 + 协议 + 管理台按钮）vs 先做服务端脚本与训练镜像手动验证，按钮下一轮再加。
- 训练触发是否限制空闲时段（如仅夜间）以进一步规避与在线语音的 CPU 争抢。
- 训练进度回传粒度：仅阶段态（排队/训练/导出/验收/完成）vs 容器日志尾随的百分比。

## 五、微调环境参考（任一开发机或容器内）

- Python 3.10+ venv：`torch`（CPU 即可）、`transformers`、`datasets`、`accelerate`、`optimum[onnxruntime]`、`onnxruntime`。
- 中文底座：华为 TinyBERT 通用权重（TinyBERT_General_4L_312D）配中文词表，或社区中文小型 BERT；分类头用自己的意图语料训练。
- 导出：`optimum-cli export onnx` → `onnxruntime.quantization` 动态 int8 → 三件套落盘。
