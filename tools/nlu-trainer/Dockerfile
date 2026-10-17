# NLU 意图分类模型训练器：从 [nlu].intents/slots 生成的语料微调中文小模型，
# 导出服务端 [nlu] 可加载的 ONNX 三件套（model.onnx int8 / tokenizer.json / labels.txt）。
# 用法（服务端 /api/nlu/model/build 会自动执行；手动：
#   docker build -t xiaozhi-nlu-trainer tools/nlu-trainer
#   docker run --rm -v $PWD/work:/work xiaozhi-nlu-trainer）
FROM python:3.11-slim

ENV PIP_NO_CACHE_DIR=1 \
    HF_ENDPOINT=https://hf-mirror.com \
    MODEL_NAME=hfl/rbt3

# CPU 版 torch（官方 cpu 轮子源，体积远小于默认 CUDA 版）+ 训练/导出栈。
RUN pip install --index-url https://download.pytorch.org/whl/cpu torch \
    && pip install "transformers>=4.44" datasets accelerate \
       "optimum[onnxruntime]>=1.19" onnxruntime sentencepiece

COPY train.py /app/train.py
WORKDIR /work
ENTRYPOINT ["python", "/app/train.py"]
