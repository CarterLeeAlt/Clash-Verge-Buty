import { useRef } from "react";
import { useLockFn } from "ahooks";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import {
  getConnections,
  providerHealthCheck,
  updateProxy,
  deleteConnection,
} from "@/services/api";
import { useProfiles } from "@/hooks/use-profiles";
import { useVerge } from "@/hooks/use-verge";
import { BaseEmpty } from "../base";
import { useRenderList } from "./use-render-list";
import { ProxyRender } from "./proxy-render";
import delayManager from "@/services/delay";

interface Props {
  mode: string;
}

export const ProxyGroups = (props: Props) => {
  const { mode } = props;

  const { renderList, onProxies, onHeadState } = useRenderList(mode);

  const { verge } = useVerge();
  const { current, patchCurrent } = useProfiles();
  const rawTimeout = Number(verge?.default_latency_timeout);
  const timeout =
    Number.isInteger(rawTimeout) && rawTimeout > 0 ? rawTimeout : 10000;

  const virtuosoRef = useRef<VirtuosoHandle>(null);

  const handleChangeProxy = useLockFn(
    async (group: IProxyGroupItem, proxy: IProxyItem) => {
      if (group.type !== "Selector" && group.type !== "Fallback") return;

      const { name, now } = group;
      await updateProxy(name, proxy.name);
      onProxies();

      if (verge?.auto_close_connection) {
        getConnections().then(({ connections }) => {
          connections.forEach((conn) => {
            if (conn.chains.includes(now!)) {
              deleteConnection(conn.id);
            }
          });
        });
      }

      if (!current) return;
      if (!current.selected) current.selected = [];

      const index = current.selected.findIndex(
        (item) => item.name === group.name
      );

      if (index < 0) {
        current.selected.push({ name, now: proxy.name });
      } else {
        current.selected[index] = { name, now: proxy.name };
      }
      await patchCurrent({ selected: current.selected });
    }
  );

  const handleCheckAll = useLockFn(async (groupName: string) => {
    const proxies = renderList
      .filter(
        (e) => e.group?.name === groupName && (e.type === 2 || e.type === 4)
      )
      .flatMap((e) => e.proxyCol || e.proxy!)
      .filter(Boolean);

    const providers = new Set(proxies.map((p) => p!.provider!).filter(Boolean));

    if (providers.size) {
      Promise.allSettled(
        [...providers].map((p) => providerHealthCheck(p))
      ).then(() => onProxies());
    }

    const names = proxies.filter((p) => !p!.provider).map((p) => p!.name);
    await delayManager.checkListDelay(names, groupName, timeout);

    onProxies();
  });

  const handleLocation = (group: IProxyGroupItem) => {
    if (!group) return;
    const { name, now } = group;

    const index = renderList.findIndex(
      (e) =>
        e.group?.name === name &&
        ((e.type === 2 && e.proxy?.name === now) ||
          (e.type === 4 && e.proxyCol?.some((p) => p.name === now)))
    );

    if (index >= 0) {
      virtuosoRef.current?.scrollToIndex?.({
        index,
        align: "center",
        behavior: "smooth",
      });
    }
  };

  if (mode === "direct") {
    return <BaseEmpty text="Direct Mode" />;
  }

  return (
    <Virtuoso
      ref={virtuosoRef}
      style={{ height: "100%" }}
      totalCount={renderList.length}
      increaseViewportBy={256}
      itemContent={(index) => (
        <ProxyRender
          key={renderList[index].key}
          item={renderList[index]}
          indent={mode === "rule" || mode === "script"}
          onLocation={handleLocation}
          onCheckAll={handleCheckAll}
          onHeadState={onHeadState}
          onChangeProxy={handleChangeProxy}
        />
      )}
    />
  );
};
