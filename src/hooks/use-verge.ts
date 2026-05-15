import useSWR, { mutate } from "swr";
import {
  getVergeConfig,
  patchVergeConfig,
  checkService,
} from "@/services/cmds";
import { getAxios } from "@/services/api";

let runtimeRefreshSeq = 0;

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForServiceActive(maxTry = 5, interval = 3000) {
  for (let i = 0; i < maxTry; i++) {
    const status = await checkService();
    await mutate("checkService", status, false);

    if (status?.api_ready) {
      return true;
    }

    if (i < maxTry - 1) {
      await sleep(interval);
    }
  }

  return false;
}

export const useVerge = () => {
  const { data: verge, mutate: mutateVerge } = useSWR(
    "getVergeConfig",
    getVergeConfig
  );

  const patchVerge = async (value: Partial<IVergeConfig>) => {
    await patchVergeConfig(value);
    await mutateVerge();

    const affectsRuntime =
      value.enable_service_mode !== undefined ||
      value.enable_tun_mode !== undefined ||
      value.enable_system_proxy !== undefined;

    if (value.enable_service_mode !== undefined) {
      if (value.enable_service_mode) {
        await waitForServiceActive(5, 3000);
      } else {
        const status = await checkService();
        await mutate("checkService", status, false);
      }

      await getAxios(true);
      await mutate("getClashInfo");
      mutate("getClashConfig");
      mutate("getProxies");
      mutate("getVersion");
      mutate("checkService");
    }

    if (affectsRuntime) {
      const seq = ++runtimeRefreshSeq;
      mutate("getClashInfo");
      mutate("getClashConfig");
      mutate("getRuntimeConfig");
      setTimeout(() => {
        if (seq !== runtimeRefreshSeq) return;
        mutate("getClashInfo");
        mutate("getClashConfig");
      }, 600);
    }
  };

  return {
    verge,
    mutateVerge,
    patchVerge,
  };
};
