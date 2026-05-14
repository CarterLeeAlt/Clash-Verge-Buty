import dayjs from "dayjs";
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useRecoilValue, useSetRecoilState } from "recoil";
import { getClashLogs } from "@/services/cmds";
import { useClashInfo } from "@/hooks/use-clash";
import { atomEnableLog, atomLogData, atomLogError } from "@/services/states";
import { useWebsocket } from "@/hooks/use-websocket";

const MAX_LOG_NUM = 2000;

// setup the log websocket
export const useLogSetup = () => {
  const { clashInfo } = useClashInfo();

  const enableLog = useRecoilValue(atomEnableLog);
  const setLogData = useSetRecoilState(atomLogData);
  const setLogError = useSetRecoilState(atomLogError);

  const { connect, disconnect } = useWebsocket((event) => {
    try {
      const data = JSON.parse(event.data) as ILogItem;
      const time = dayjs().format("MM-DD HH:mm:ss");
      setLogData((l) => {
        const next = [{ ...data, time }, ...l];
        return next.slice(0, MAX_LOG_NUM);
      });
    } catch {}
  }, {
    onError: () => {
      setLogError("Log websocket disconnected or external-controller unavailable");
    },
  });

  useEffect(() => {
    let unlisten: null | (() => void) = null;

    listen<ILogItem>("verge://app-log", (event) => {
      const data = event.payload;
      const time = dayjs().format("MM-DD HH:mm:ss");
      setLogData((l) => {
        const next = [{ ...data, time }, ...l];
        return next.slice(0, MAX_LOG_NUM);
      });
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {
        setLogError("Failed to subscribe app log stream");
      });

    if (!enableLog) {
      setLogError("Log stream paused");
      return () => {
        unlisten?.();
      };
    }

    if (!clashInfo?.server) {
      setLogError("Core not running or external-controller is not ready");
      return;
    }

    setLogError(null);

    getClashLogs()
      .then((logs) => setLogData(logs.reverse().slice(0, MAX_LOG_NUM)))
      .catch(() => setLogError("Failed to load historical logs"));

    const { server = "", secret = "" } = clashInfo;
    connect(`ws://${server}/logs?token=${encodeURIComponent(secret)}`);

    return () => {
      disconnect();
      unlisten?.();
    };
  }, [clashInfo, enableLog, setLogData, setLogError]);
};
