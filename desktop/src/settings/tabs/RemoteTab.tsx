import { Fragment, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { PixelButton } from "../../components/PixelButton";
import * as ipc from "../../lib/ipc";
import type { RemoteHostSetting } from "../../lib/types";
import {
  SettingRow,
  SettingsSection,
  TextField,
  Toggle,
  useControlId,
  type SettingsTabProps,
} from "../controls";

const EMPTY_HOST: RemoteHostSetting = {
  id: "",
  name: "",
  host: "",
  port: null,
  user: "",
  key_path: "",
  auth_socket: "",
  cwd_filter: "",
  auto_connect: false,
};

function RemoteHostActions({ host }: { host: RemoteHostSetting }) {
  const { t } = useTranslation();
  const localId = useId();
  const remoteHostId = useId();
  const remotePortId = useId();
  const [localPort, setLocalPort] = useState("0");
  const [remoteHost, setRemoteHost] = useState("127.0.0.1");
  const [remotePort, setRemotePort] = useState("3000");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const forward = async () => {
    setBusy(true);
    setError(null);
    try {
      const bound = await ipc.sshForward(
        host.id,
        Number(localPort) || 0,
        remoteHost.trim() || "127.0.0.1",
        Number(remotePort) || 0,
      );
      setMessage(t("settings.remote.forwardListening", { port: bound }));
    } catch (cause) {
      setMessage(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const deploy = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await ipc.sshDeployHook(host.id);
      setMessage(result.summary ||
        t("settings.remote.deployDone", { path: result.remote_wrapper || result.remote_binary }));
    } catch (cause) {
      setMessage(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const uninstall = async () => {
    setBusy(true);
    setError(null);
    try {
      setMessage(await ipc.sshUninstallHooks(host.id));
    } catch (cause) {
      setMessage(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="border-b border-white/[0.06] px-3.5 pb-2.5 last:border-b-0">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[11px] text-white/45">
          {t("settings.remote.forwardPort")}
        </span>
        <TextField
          id={localId}
          value={localPort}
          onChange={setLocalPort}
          className="w-16"
        />
        <span className="text-[11px] text-white/40">→</span>
        <TextField
          id={remoteHostId}
          value={remoteHost}
          onChange={setRemoteHost}
          className="w-32"
        />
        <span className="text-[11px] text-white/40">:</span>
        <TextField
          id={remotePortId}
          value={remotePort}
          onChange={setRemotePort}
          className="w-16"
        />
        <PixelButton disabled={busy} onClick={() => void forward()}>
          {t("settings.remote.forward")}
        </PixelButton>
        <PixelButton disabled={busy} onClick={() => void deploy()}>
          {t("settings.remote.deployHook")}
        </PixelButton>
        <PixelButton variant="ghost" disabled={busy} onClick={() => void uninstall()}>
          {t("settings.remote.uninstallHooks")}
        </PixelButton>
      </div>
      {message && (
        <div className="mt-1 break-all text-[11px] text-[#6ee7a8]">
          {message}
        </div>
      )}
      {error && (
        <div className="mt-1 break-all text-[11px] text-[#ff7b7b]">{error}</div>
      )}
    </div>
  );
}

export function RemoteTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const nameId = useControlId("remote-name");
  const hostId = useControlId("remote-host");
  const portId = useControlId("remote-port");
  const userID = useControlId("remote-user");
  const keyId = useControlId("remote-key");
  const agentSocketId = useControlId("remote-auth-socket");
  const cwdFilterId = useControlId("remote-cwd-filter");
  const autoConnectId = useControlId("remote-auto-connect");
  const hosts = settings.remote_hosts;
  const [adding, setAdding] = useState(false);
  const [editingFilter, setEditingFilter] = useState<string | null>(null);
  const [filterDraft, setFilterDraft] = useState("");
  const [autoConnectDraft, setAutoConnectDraft] = useState(false);
  const [agentSocketDraft, setAgentSocketDraft] = useState("");
  const [draft, setDraft] = useState<RemoteHostSetting>(EMPTY_HOST);
  const [statuses, setStatuses] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void Promise.allSettled(hosts.map(async (host) => [host.id, await ipc.sshStatus(host.id)] as const))
        .then((results) => {
          if (cancelled) return;
          const entries = results.flatMap((result) => result.status === "fulfilled" ? [result.value] : []);
          setStatuses((previous) => ({ ...previous, ...Object.fromEntries(entries) }));
        });
    };
    refresh();
    const timer = window.setInterval(refresh, 1_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [hosts]);

  const connect = async (host: RemoteHostSetting) => {
    setBusy(host.id);
    setError(null);
    setStatuses((previous) => ({ ...previous, [host.id]: "connecting" }));
    try {
      const status = await ipc.sshConnect(host.id);
      setStatuses((previous) => ({ ...previous, [host.id]: status }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async (host: RemoteHostSetting) => {
    setBusy(host.id);
    try {
      await ipc.sshDisconnect(host.id);
      setStatuses((previous) => ({ ...previous, [host.id]: "not_connected" }));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const addHost = () => {
    if (!draft.host.trim()) return;
    onChange({
      remote_hosts: [
        ...hosts,
        {
          ...draft,
          id: `${Date.now()}`,
          name: draft.name.trim() || draft.host.trim(),
        },
      ],
    });
    setDraft(EMPTY_HOST);
    setAdding(false);
  };

  const removeHost = async (id: string) => {
    await ipc.sshDisconnect(id);
    onChange({ remote_hosts: hosts.filter((host) => host.id !== id) });
  };

  return (
    <div>
      <SettingsSection title={t("settings.remote.title")}>
        {hosts.length === 0 && !adding && (
          <div className="px-3.5 py-4 text-[12px] text-white/45">
            {t("settings.remote.empty")}
          </div>
        )}
        {hosts.map((host) => {
          const status = statuses[host.id];
          const active = status === "connected" || status === "connecting" || status === "reconnecting" || status === "waiting_network";
          const statusText = status === "waiting_network" ? "waitingNetwork" : active ? status : "notConnected";
          return (
          <Fragment key={host.id}>
            <SettingRow
              htmlFor={`remote-${host.id}`}
              label={`${host.name} · ${host.user ? `${host.user}@` : ""}${host.host}${host.port ? `:${host.port}` : ""}`}
              description={t(`settings.remote.${statusText}`)}
            >
              {active ? (
                <PixelButton
                  variant="ghost"
                  disabled={busy === host.id}
                  onClick={() => void disconnect(host)}
                >
                  {t("settings.remote.disconnect")}
                </PixelButton>
              ) : (
                <PixelButton
                  variant="primary"
                  disabled={busy === host.id}
                  onClick={() => void connect(host)}
                >
                  {t("settings.remote.connect")}
                </PixelButton>
              )}
              <PixelButton variant="ghost" onClick={() => {
                setEditingFilter(editingFilter === host.id ? null : host.id);
                setFilterDraft(host.cwd_filter);
                setAutoConnectDraft(host.auto_connect);
                setAgentSocketDraft(host.auth_socket);
              }}>
                {t("settings.remote.cwdFilter")}
              </PixelButton>
              <PixelButton variant="ghost" onClick={() => void removeHost(host.id).catch((cause: unknown) => setError(String(cause)))}>
                {t("settings.remote.remove")}
              </PixelButton>
            </SettingRow>
            {editingFilter === host.id && (
              <>
              <SettingRow htmlFor={`${cwdFilterId}-edit`} label={t("settings.remote.cwdFilter")}
                description={t("settings.remote.cwdFilterHint")}>
                <TextField id={`${cwdFilterId}-edit`} value={filterDraft}
                  placeholder={t("settings.remote.cwdFilterPlaceholder")} onChange={setFilterDraft} />
                <PixelButton onClick={() => {
                  onChange({ remote_hosts: hosts.map((item) => item.id === host.id
                    ? { ...item, cwd_filter: filterDraft.trim(), auto_connect: autoConnectDraft, auth_socket: agentSocketDraft.trim() }
                    : item) });
                  setEditingFilter(null);
                }}>
                  {t("common.save")}
                </PixelButton>
              </SettingRow>
              <SettingRow htmlFor={`${autoConnectId}-edit`} label={t("settings.remote.autoConnect")}>
                <Toggle id={`${autoConnectId}-edit`} label={t("settings.remote.autoConnect")}
                  checked={autoConnectDraft} onChange={setAutoConnectDraft} />
              </SettingRow>
              <SettingRow htmlFor={`${agentSocketId}-edit`} label={t("settings.remote.authSocket")}>
                <TextField id={`${agentSocketId}-edit`} value={agentSocketDraft}
                  placeholder={"\\\\.\\pipe\\openssh-ssh-agent"} onChange={setAgentSocketDraft} />
              </SettingRow>
              </>
            )}
            {statuses[host.id] === "connected" && (
              <RemoteHostActions host={host} />
            )}
          </Fragment>
          );
        })}
        {!adding && (
          <div className="px-3.5 py-2">
            <PixelButton onClick={() => setAdding(true)}>
              {t("settings.remote.addHost")}
            </PixelButton>
          </div>
        )}
        {error && (
          <div className="px-3.5 pb-2 text-[11px] text-[#ff7b7b]">{error}</div>
        )}
      </SettingsSection>

      {adding && (
        <SettingsSection title={t("settings.remote.newHost")}>
          <SettingRow htmlFor={nameId} label={t("settings.remote.name")}>
            <TextField
              id={nameId}
              value={draft.name}
              placeholder="Build box"
              onChange={(name) => setDraft((value) => ({ ...value, name }))}
            />
          </SettingRow>
          <SettingRow htmlFor={hostId} label={t("settings.remote.host")}>
            <TextField
              id={hostId}
              value={draft.host}
              placeholder="host.example.com"
              onChange={(host) => setDraft((value) => ({ ...value, host }))}
            />
          </SettingRow>
          <SettingRow htmlFor={portId} label={t("settings.remote.port")}>
            <TextField
              id={portId}
              value={draft.port?.toString() ?? ""}
              placeholder="22"
              className="w-16 text-right tabular-nums"
              onChange={(raw) => {
                if (/^\d*$/.test(raw) && Number(raw) <= 65535) {
                  setDraft((value) => ({ ...value, port: Number(raw) || null }));
                }
              }}
            />
          </SettingRow>
          <SettingRow htmlFor={userID} label={t("settings.remote.user")}>
            <TextField
              id={userID}
              value={draft.user}
              placeholder="root"
              onChange={(user) => setDraft((value) => ({ ...value, user }))}
            />
          </SettingRow>
          <SettingRow htmlFor={keyId} label={t("settings.remote.keyPath")}>
            <TextField
              id={keyId}
              value={draft.key_path}
              placeholder="~/.ssh/id_ed25519"
              onChange={(key_path) => setDraft((value) => ({ ...value, key_path }))}
            />
          </SettingRow>
          <SettingRow htmlFor={agentSocketId} label={t("settings.remote.authSocket")}>
            <TextField id={agentSocketId} value={draft.auth_socket}
              placeholder={"\\\\.\\pipe\\openssh-ssh-agent"}
              onChange={(auth_socket) => setDraft((value) => ({ ...value, auth_socket }))} />
          </SettingRow>
          <SettingRow htmlFor={cwdFilterId} label={t("settings.remote.cwdFilter")}
            description={t("settings.remote.cwdFilterHint")}>
            <TextField id={cwdFilterId} value={draft.cwd_filter}
              placeholder={t("settings.remote.cwdFilterPlaceholder")}
              onChange={(cwd_filter) => setDraft((value) => ({ ...value, cwd_filter }))} />
          </SettingRow>
          <SettingRow htmlFor={autoConnectId} label={t("settings.remote.autoConnect")}>
            <Toggle id={autoConnectId} label={t("settings.remote.autoConnect")}
              checked={draft.auto_connect}
              onChange={(auto_connect) => setDraft((value) => ({ ...value, auto_connect }))} />
          </SettingRow>
          <div className="flex justify-end gap-2 px-3.5 py-2">
            <PixelButton variant="ghost" onClick={() => setAdding(false)}>
              {t("common.cancel")}
            </PixelButton>
            <PixelButton variant="primary" onClick={addHost}>
              {t("settings.remote.add")}
            </PixelButton>
          </div>
        </SettingsSection>
      )}
    </div>
  );
}
