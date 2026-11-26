// Hand-maintained companion for ts-rs's serde_json::Value mapping.
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
