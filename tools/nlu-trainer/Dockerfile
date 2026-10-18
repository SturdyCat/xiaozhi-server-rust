# NLU 专用训练镜像：Python 依赖在**镜像构建期**全部装好，训练容器启动即跑，
# 无运行期 pip 引导。阿里云 ACR 自动构建（Dockerfile 路径指向本文件，构建目录 tools/nlu-trainer）：
#   镜像：registry.cn-hangzhou.aliyuncs.com/notip-com-cn/nlu-trainer:<tag>
# 服务端（nlu_build.rs）默认按该 tag `docker pull` + `docker run`。
FROM python:3.11-slim

# PyPI 阿里云镜像作主 index（全量包；ACR 云端构建环境访问清华源会拿到空列表，
# 阿里源与 ACR 同云最稳），CPU 轮子仓库作 extra index（只收录 torch 系包，
# 不能当唯一 index，否则 transformers 等报 from versions: none）。失败回退官方 PyPI。
COPY requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir --retries 5 --timeout 60 \
      -i https://mirrors.aliyun.com/pypi/simple/ \
      --extra-index-url https://download.pytorch.org/whl/cpu \
      -r /tmp/requirements.txt \
 || pip install --no-cache-dir \
      --extra-index-url https://download.pytorch.org/whl/cpu \
      -r /tmp/requirements.txt \
 && rm -f /tmp/requirements.txt

# HF 权重经国内镜像拉取；训练脚本固定路径，工作目录与产物目录由服务端挂载。
ENV HF_ENDPOINT=https://hf-mirror.com \
    MODEL_NAME=hfl/rbt3
COPY train.py /opt/train.py
# 底座权重烘进镜像（hfl/rbt3 约 40MB）：hf-mirror 时有抖动，预热后训练完全离线可跑，
# 网络失败时 transformers 自动回退本地缓存；主镜像源失败换官方源重试一次。
RUN sh -c 'python -c "from huggingface_hub import snapshot_download; snapshot_download(\"hfl/rbt3\")"' \
 || HF_ENDPOINT=https://huggingface.co \
    python -c "from huggingface_hub import snapshot_download; snapshot_download('hfl/rbt3')"
ENTRYPOINT ["python", "/opt/train.py"]
