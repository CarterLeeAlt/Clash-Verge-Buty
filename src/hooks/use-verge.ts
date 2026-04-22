import useSWR, { mutate } from "swr";
import {
  getVergeConfig,
  patchVergeConfig,
  checkService,
} from "@/services/cmds";

function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForServiceActive(maxTry = 5, interval = 3000) {
  for (let i = 0; i < maxTry; i++) {
    const status = await checkService();

    // 立即把最新状态写回 SWR 缓存，所有用到 "checkService" 的组件都会同步更新
    await mutate("checkService", status, false);

    if (status === "active") {
      return true;
    }

    // 最后一次就不用再等了
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

    // 只有切换 service mode 时，才主动处理 service 状态同步
    if (value.enable_service_mode !== undefined) {
      if (value.enable_service_mode) {
        // 开启服务模式后，每 3 秒检查一次，最多 5 次
        await waitForServiceActive(5, 3000);
      } else {
        // 关闭时直接刷新一次即可
        const status = await checkService();
        await mutate("checkService", status, false);
      }
    }
  };

  return {
    verge,
    mutateVerge,
    patchVerge,
  };
};
