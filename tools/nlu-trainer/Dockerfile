# NLU 专用训练镜像：Python 依赖在**镜像构建期**全部装好，训练容器启动即跑，
# 无运行期 pip 引导。阿里云 ACR 自动构建（Dockerfile 路径指向本文件，构建目录 tools/nlu-trainer）：
#   镜像：registry.cn-hangzhou.aliyuncs.com/notip-com-cn/nlu-trainer:<tag>
# 新服务仅在依赖镜像缺失时 pull；固定 image id 后以 python -c 执行服务内嵌脚本。
# 本镜像自带 /opt/train.py 供 standalone 使用；独立运行该入口需重建镜像才能更新脚本。
FROM python:3.11-slim

# 阿里云 PyPI 作主 index（全量包；ACR 云端构建环境访问清华源会拿到空列表，
# 阿里源与 ACR 同云最稳），其余依赖从这里装。
# ⚠️ torch 必须单独从 pytorch 的【CPU 轮子仓库】用 --index-url（作主索引）安装：
#   若把 aliyun 当主索引、cpu 当 extra，pip 会优先命中 aliyun 镜像里的【CUDA 版】
#   torch（自带 CUDA 运行时，解包 ~2GB），cpu 仓库只是 fallback 不会生效——这正是
#   镜像此前虚胖的主因。CPU 轮子不含 CUDA，解包仅约 0.5GB。锁 2.4.1 与
#   transformers<4.44 兼容。
COPY requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir --retries 5 --timeout 60 \
      --index-url https://download.pytorch.org/whl/cpu \
      torch==2.4.1 \
 && pip install --no-cache-dir --retries 5 --timeout 60 \
      -i https://mirrors.aliyun.com/pypi/simple/ \
      --extra-index-url https://download.pytorch.org/whl/cpu \
      -r /tmp/requirements.txt \
 || pip install --no-cache-dir \
      --extra-index-url https://download.pytorch.org/whl/cpu \
      -r /tmp/requirements.txt \
 && rm -f /tmp/requirements.txt

# HF 权重经国内镜像拉取；训练脚本固定路径，工作目录与产物目录由服务端挂载。
# MODEL_NAME 这里是**规范名**（当前唯一底座 minirbt-h288），不是 hub id：train.py 的 BASES 表
# 负责把规范名映射到 hub 与微调超参。服务端每次 build 都用
# `docker run -e MODEL_NAME=... -e LEARNING_RATE=... -e EPOCHS=...` 覆盖；
# 此值仅供 `docker run <image>` 这类 standalone 运行的缺省臂，**必须**等于
# `base.rs` 的 BASE_MODEL 与 train.py 的 BASE_NAME（有单测用 include_str! 锁住）。
ENV HF_ENDPOINT=https://hf-mirror.com \
    MODEL_NAME=minirbt-h288
COPY train.py /opt/train.py
# 底座权重烘进镜像，训练容器完全离线可跑。allow_patterns 只取权重与
# 词表/配置：minirbt-h288 仓库还挂着 tf_model.h5 / flax 权重（各约 150MB），
# 全量 snapshot 会把镜像白撑大且训练用不到。同时收 pytorch_model.bin 与
# model.safetensors 两种权重格式——只发 safetensors 的仓库否则会静默回落到
# 运行期下载，离线训练的承诺就破了。
# minirbt-h288：6 层 miniRBT，仓库只有 vocab.txt（无 tokenizer.json），
# 训练期靠 transformers 的 slow→fast 自动转换产出 fast 分词器。
RUN sh -c 'python -c "from huggingface_hub import snapshot_download; snapshot_download(\"hfl/minirbt-h288\", allow_patterns=[\"*.json\", \"*.txt\", \"pytorch_model.bin\", \"model.safetensors\"])"' \
    || HF_ENDPOINT=https://huggingface.co \
    python -c "from huggingface_hub import snapshot_download; snapshot_download('hfl/minirbt-h288', allow_patterns=['*.json', '*.txt', 'pytorch_model.bin', 'model.safetensors'])"
ENTRYPOINT ["python", "/opt/train.py"]
