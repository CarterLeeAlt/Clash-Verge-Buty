import dayjs from "dayjs";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { useRecoilValue, useSetRecoilState } from "recoil";
import { getClashLogs } from "@/services/cmds";
import { useClashInfo } from "@/hooks/use-clash";
import {
  atomEnableLog,
  atomLogConnState,
  atomLogData,
  atomLogError,
} from "@/services/states";
import { useWebsocket } from "@/hooks/use-websocket";
import { buildControllerWsUrl } from "@/utils/controller";

const MAX_LOG_NUM = 2000;
const FALLBACK_SYNC_INTERVAL = 1500;
const RECONNECTING_HINT_DELAY = 8000;
const getLogKey = (item: Partial<ILogItem>) => {
  const payload = item.payload ?? (item as any).message ?? (item as any).msg ?? "";
  return `${item.time ?? ""}|${item.type ?? ""}|${payload}`;
};

const normalizeLogTime = (time?: string) => {
  if (!time) return dayjs().format("MM-DD HH:mm:ss");
  const parsed = dayjs(time);
  return parsed.isValid() ? parsed.format("MM-DD HH:mm:ss") : time;
};

export const useLogSetup = () => {
  const { clashInfo } = useClashInfo();
  const enableLog = useRecoilValue(atomEnableLog);
  const setLogData = useSetRecoilState(atomLogData);
  const setLogError = useSetRecoilState(atomLogError);
  const setLogConnState = useSetRecoilState(atomLogConnState);

  const historyReqIdRef = useRef(0);
  const fallbackTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectHintTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const wsUrl = useMemo(
    () => buildControllerWsUrl(clashInfo?.server || "", "/logs", clashInfo?.secret || ""),
    [clashInfo?.server, clashInfo?.secret],
  );

  const clearFallbackTimer = useCallback(() => {
    if (!fallbackTimerRef.current) return;
    clearTimeout(fallbackTimerRef.current);
    fallbackTimerRef.current = null;
  }, []);

  const clearReconnectHintTimer = useCallback(() => {
    if (!reconnectHintTimerRef.current) return;
    clearTimeout(reconnectHintTimerRef.current);
    reconnectHintTimerRef.current = null;
  }, []);

  const enterReconnectingState = useCallback(() => {
    setLogConnState("reconnecting");
    clearReconnectHintTimer();
    reconnectHintTimerRef.current = setTimeout(() => {
      setLogError("日志连接恢复中…");
    }, RECONNECTING_HINT_DELAY);
  }, [clearReconnectHintTimer, setLogConnState, setLogError]);

  const mergeLogs = useCallback(
    (incoming: ILogItem[]) => {
      setLogData((old) => {
        const map = new Map<string, ILogItem>();
        [...incoming, ...old].forEach((item) => map.set(getLogKey(item), item));
        return Array.from(map.values()).slice(0, MAX_LOG_NUM);
      });
    },
    [setLogData],
  );

  const pullHistory = useCallback(async () => {
    const reqId = ++historyReqIdRef.current;
    try {
      const logs = await getClashLogs();
      if (reqId !== historyReqIdRef.current) return;
      mergeLogs(
        (logs || []).map((item) => ({
          ...item,
          time: normalizeLogTime(item.time),
        })).reverse(),
      );
      setLogError(null);
      setLogConnState("connected");
      clearReconnectHintTimer();
      clearFallbackTimer();
    } catch (err) {
      if (reqId !== historyReqIdRef.current) return;
      console.warn("[log] fallback sync failed", err);
    }
  }, [clearFallbackTimer, clearReconnectHintTimer, mergeLogs, setLogConnState, setLogError]);

  const scheduleFallbackSync = useCallback(() => {
    clearFallbackTimer();
    if (!enableLog) return;

    fallbackTimerRef.current = setTimeout(async () => {
      fallbackTimerRef.current = null;
      await pullHistory();
      scheduleFallbackSync();
    }, FALLBACK_SYNC_INTERVAL);
  }, [clearFallbackTimer, enableLog, pullHistory]);

  const { connect, disconnect } = useWebsocket(
    (event) => {
      try {
        const data = JSON.parse(event.data) as ILogItem;
        mergeLogs([{ ...data, time: normalizeLogTime(data.time) }]);
      } catch {
        // ignore invalid event payload
      }
    },
    {
      reconnect: true,
      maxRetries: "infinite",
      retryInterval: 500,
      maxRetryInterval: 10000,
      backoff: true,
      onOpen: () => {
        setLogConnState("connected");
        setLogError(null);
        pullHistory();
        clearFallbackTimer();
        clearReconnectHintTimer();
      },
      onClose: () => {
        enterReconnectingState();
        pullHistory();
        scheduleFallbackSync();
      },
      onError: () => {
        enterReconnectingState();
        pullHistory();
        scheduleFallbackSync();
      },
    },
  );

  useEffect(() => {
    let unlisten: null | (() => void) = null;
    listen<ILogItem>("verge://app-log", (event) => {
      mergeLogs([{ ...event.payload, time: normalizeLogTime(event.payload?.time) }]);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => {
        console.warn("[log] subscribe app log failed", err);
      });

    return () => unlisten?.();
  }, [mergeLogs]);

  useEffect(() => {
    let unlisten: null | (() => void) = null;
    listen("verge://refresh-clash-config", () => {
      if (!enableLog) return;
      enterReconnectingState();
      pullHistory();
      if (wsUrl) {
        disconnect();
        connect(wsUrl);
      }
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => {
        console.warn("[log] subscribe refresh-clash-config failed", err);
      });

    return () => unlisten?.();
  }, [connect, disconnect, enableLog, enterReconnectingState, pullHistory, wsUrl]);

  useEffect(() => {
    if (!enableLog) {
      historyReqIdRef.current += 1;
      clearFallbackTimer();
      clearReconnectHintTimer();
      disconnect();
      setLogConnState("paused");
      setLogError("Log stream paused");
      return;
    }

    if (!wsUrl) {
      historyReqIdRef.current += 1;
      enterReconnectingState();
      disconnect();
      setLogError("Core not running or external-controller is not ready");
      pullHistory();
      scheduleFallbackSync();
      return;
    }

    setLogConnState("reconnecting");
    connect(wsUrl);
    pullHistory();

    return () => {
      historyReqIdRef.current += 1;
      clearFallbackTimer();
      clearReconnectHintTimer();
      disconnect();
    };
  }, [clearFallbackTimer, clearReconnectHintTimer, connect, disconnect, enableLog, enterReconnectingState, pullHistory, scheduleFallbackSync, setLogConnState, setLogError, wsUrl]);
};
