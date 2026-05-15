import dayjs from "dayjs";
import { useEffect, useMemo, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { useRecoilValue, useSetRecoilState } from "recoil";
import { getClashLogs } from "@/services/cmds";
import { useClashInfo } from "@/hooks/use-clash";
import { atomEnableLog, atomLogData, atomLogError } from "@/services/states";
import { useWebsocket } from "@/hooks/use-websocket";

const MAX_LOG_NUM = 2000;
const getLogKey = (item: Partial<ILogItem>) =>
  `${item.time ?? ""}|${item.type ?? ""}|${item.payload ?? ""}`;

// setup the log websocket
export const useLogSetup = () => {
  const { clashInfo } = useClashInfo();

  const enableLog = useRecoilValue(atomEnableLog);
  const setLogData = useSetRecoilState(atomLogData);
  const setLogError = useSetRecoilState(atomLogError);
  const historyReqIdRef = useRef(0);

  const wsUrl = useMemo(() => {
    if (!clashInfo?.server) return "";
    return `ws://${clashInfo.server}/logs?token=${encodeURIComponent(clashInfo.secret || "")}`;
  }, [clashInfo?.server, clashInfo?.secret]);

  const mergeLogs = (incoming: ILogItem[]) => {
    setLogData((old) => {
      const map = new Map<string, ILogItem>();
      [...incoming, ...old].forEach((item) => {
        map.set(getLogKey(item), item);
      });
      return Array.from(map.values()).slice(0, MAX_LOG_NUM);
    });
  };

  const pullHistory = async () => {
    const reqId = ++historyReqIdRef.current;
    try {
      const logs = await getClashLogs();
      if (reqId !== historyReqIdRef.current) return;
      mergeLogs((logs || []).map((item) => ({ ...item, time: item.time || dayjs().format("MM-DD HH:mm:ss") })).reverse());
      setLogError(null);
    } catch {
      if (reqId !== historyReqIdRef.current) return;
      setLogError("Failed to load historical logs");
    }
  };

  const { connect, disconnect } = useWebsocket(
    (event) => {
      try {
        const data = JSON.parse(event.data) as ILogItem;
        const time = dayjs().format("MM-DD HH:mm:ss");
        mergeLogs([{ ...data, time }]);
      } catch {}
    },
    {
      retryInterval: 1200,
      onOpen: () => {
        setLogError(null);
        pullHistory();
      },
      onError: () => {
        setLogError("Log websocket disconnected or external-controller unavailable");
      },
      onClose: () => {
        setLogError("Log websocket disconnected or external-controller unavailable");
      },
    }
  );

  useEffect(() => {
    let unlisten: null | (() => void) = null;

    listen<ILogItem>("verge://app-log", (event) => {
      const data = event.payload;
      const time = dayjs().format("MM-DD HH:mm:ss");
      mergeLogs([{ ...data, time }]);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        setLogError("Failed to subscribe app log stream");
      });

    return () => {
      unlisten?.();
    };
  }, [setLogError]);

  useEffect(() => {
    if (!enableLog) {
      historyReqIdRef.current += 1;
      disconnect();
      setLogError("Log stream paused");
      return;
    }

    if (!wsUrl) {
      historyReqIdRef.current += 1;
      disconnect();
      setLogError("Core not running or external-controller is not ready");
      return;
    }

    connect(wsUrl);

    return () => {
      historyReqIdRef.current += 1;
      disconnect();
    };
  }, [connect, disconnect, enableLog, wsUrl, setLogError]);
};
