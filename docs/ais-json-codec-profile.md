# AIS JSON Codec Profile (`ais-json/1`)

本文件固定 AGT007A 的 JSON 表示，供 SDK/Runner/Agent 互操作使用。

## 目标

- 在 JSON/JSONL 中稳定传输 `BigInt`、`Uint8Array`、`Error`。
- 避免跨进程或跨语言时出现多种等价表示导致的语义漂移。
- 提供可选 strict 拒绝策略（`undefined`、非有限数字）。

## 唯一表示

- `BigInt`:
  - `{"__ais_json_type":"bigint","value":"123"}`
  - `value` 必须是十进制字符串。
- `Uint8Array`:
  - `{"__ais_json_type":"uint8array","encoding":"base64","value":"..."}`
  - 仅允许 `base64` 编码。
- `Error`:
  - `{"__ais_json_type":"error","name":"ErrorName","message":"...","stack":"..."?}`
  - 默认不输出 `stack`（隐私默认）。
  - 仅在 `include_error_stack: true` 时输出 `stack`。

## Strict 拒绝策略

- SDK 提供可选 strict 编码参数：
  - `reject_undefined: true` 时，遇到 `undefined` 抛错。
  - `reject_non_finite_number: true` 时，遇到 `NaN/Infinity` 抛错。
- 默认不启用 strict 拒绝，以兼容现有对象序列化习惯；调用方可按场景开启。

## 统一入口

- SDK:
  - `stringifyAisJson` / `parseAisJson`
  - `aisJsonCodec`
  - `AIS_JSON_CODEC_PROFILE`
- Runner:
  - JSONL 输出（events/trace）走 SDK codec。
  - JSONL 输入（commands）走 SDK codec。
  - `--inputs/--ctx/--args/--retry` 解析改为走 SDK codec。
