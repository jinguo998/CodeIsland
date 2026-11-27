// The sidecar connects to the current user's named pipe. execFile accepts the
// executable and arguments separately, including paths with spaces.
function runHook(payload/*payload*/, timeoutMs/*millis*/, env/*env*/ = undefined) {
  const name = Reflect.get(payload, "hook_event_name") || Reflect.get(payload, "hookEventName");
  let child/*child*/;
  const promise = new Promise/*result*/((resolve) => {
    if (!name) { resolve(null); return; }
    const args = ["send", "--source", __CODEISLAND_SOURCE__, "--event", name,
      "--timeout-ms", String(timeoutMs), "--stdin"];
    try {
      child = execFile(__CODEISLAND_EXECUTABLE__, args,
        { timeout: timeoutMs + 2000, maxBuffer: 1048576, windowsHide: true, env },
        (error, stdout) => {
          if (error) { resolve(null); return; }
          try { resolve(JSON.parse(stdout || "{}")); } catch { resolve(null); }
        });
      child.stdin?.on("error", () => resolve(null));
      child.stdin?.end(JSON.stringify(payload));
    } catch { resolve(null); }
  });
  return { promise, cancel: () => { child?.kill(); } };
}
function sendToSocket(payload/*payload*/) {
  return runHook(payload, 5000).promise.then((response) => response !== null);
}
function sendAndWaitResponseCancellable(payload/*payload*/, timeoutMs = 30000) {
  return runHook(payload, timeoutMs);
}
function sendAndWaitResponse(payload/*payload*/, timeoutMs = __CODEISLAND_WAIT_MS__, env/*env*/ = undefined) {
  return runHook(payload, timeoutMs, env).promise;
}
