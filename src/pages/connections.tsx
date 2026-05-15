import { useEffect, useMemo, useRef, useState } from "react";
import { useLockFn } from "ahooks";
import {
  Box,
  Button,
  IconButton,
  MenuItem,
  Paper,
  Select,
  TextField,
} from "@mui/material";
import { useRecoilState } from "recoil";
import { Virtuoso } from "react-virtuoso";
import { useTranslation } from "react-i18next";
import { TableChartRounded, TableRowsRounded } from "@mui/icons-material";
import { closeAllConnections, getConnections } from "@/services/api";
import { atomConnectionSetting } from "@/services/states";
import { useClashInfo } from "@/hooks/use-clash";
import { BaseEmpty, BasePage } from "@/components/base";
import { useWebsocket } from "@/hooks/use-websocket";
import { ConnectionItem } from "@/components/connection/connection-item";
import { ConnectionTable } from "@/components/connection/connection-table";
import {
  ConnectionDetail,
  ConnectionDetailRef,
} from "@/components/connection/connection-detail";
import parseTraffic from "@/utils/parse-traffic";

const initConn = { uploadTotal: 0, downloadTotal: 0, connections: [] };

type OrderFunc = (list: IConnectionsItem[]) => IConnectionsItem[];

const getActiveSpeed = (conn: IConnectionsItem) =>
  (conn.curDownload ?? 0) + (conn.curUpload ?? 0);

const getTotalTraffic = (conn: IConnectionsItem) =>
  (conn.download ?? 0) + (conn.upload ?? 0);

const getStartTimestamp = (conn: IConnectionsItem) => {
  if (!conn.start) return 0;
  const ts = new Date(conn.start).getTime();
  return Number.isNaN(ts) ? 0 : ts;
};

const sortByActiveSpeed = (list: IConnectionsItem[]) =>
  list
    .map((item, index) => ({ item, index }))
    .sort((a, b) => {
      const activeDiff = getActiveSpeed(b.item) - getActiveSpeed(a.item);
      if (activeDiff !== 0) return activeDiff;

      const trafficDiff = getTotalTraffic(b.item) - getTotalTraffic(a.item);
      if (trafficDiff !== 0) return trafficDiff;

      const startDiff = getStartTimestamp(b.item) - getStartTimestamp(a.item);
      if (startDiff !== 0) return startDiff;

      return a.index - b.index;
    })
    .map(({ item }) => item);

const ConnectionsPage = () => {
  const { t, i18n } = useTranslation();
  const { clashInfo } = useClashInfo();

  const [filterText, setFilterText] = useState("");
  const [curOrderOpt, setOrderOpt] = useState("Active Speed");
  const [connData, setConnData] = useState<IConnections>(initConn);

  const [setting, setSetting] = useRecoilState(atomConnectionSetting);

  const isTableLayout = setting.layout === "table";

  const orderOpts: Record<string, OrderFunc> = {
    "Active Speed": sortByActiveSpeed,
    Default: (list) => list,
    "Upload Speed": (list) =>
      [...list].sort((a, b) => (b.curUpload ?? 0) - (a.curUpload ?? 0)),
    "Download Speed": (list) =>
      [...list].sort((a, b) => (b.curDownload ?? 0) - (a.curDownload ?? 0)),
  };

  const [filterConn, download, upload] = useMemo(() => {
    const orderFunc = orderOpts[curOrderOpt];
    let connections = connData.connections.filter((conn) =>
      (conn.metadata.host || conn.metadata.destinationIP)?.includes(filterText)
    );

    if (orderFunc) connections = orderFunc(connections);
    let download = 0;
    let upload = 0;
    connections.forEach((x) => {
      download += x.download;
      upload += x.upload;
    });
    return [connections, download, upload];
  }, [connData, filterText, curOrderOpt]);

  const syncConnections = useLockFn(async () => {
    const snapshot = await getConnections();
    const incoming = snapshot?.connections ?? [];
    setConnData((old) => {
      const oldConn = old.connections;
      const connections: IConnectionsItem[] = [];
      const rest = incoming.filter((each) => {
        const index = oldConn.findIndex((o) => o.id === each.id);
        if (index >= 0 && index < incoming.length) {
          const prev = oldConn[index];
          each.curUpload = (each.upload ?? 0) - (prev.upload ?? 0);
          each.curDownload = (each.download ?? 0) - (prev.download ?? 0);
          connections[index] = each;
          return false;
        }
        return true;
      });
      for (let i = 0; i < incoming.length; ++i) {
        if (!connections[i] && rest.length > 0) {
          const item = rest.shift()!;
          item.curUpload = item.curUpload ?? 0;
          item.curDownload = item.curDownload ?? 0;
          connections[i] = item;
        }
      }
      return { ...snapshot, connections };
    });
  });

  const { connect, disconnect } = useWebsocket(
    (event) => {
      // meta v1.15.0 出现data.connections为null的情况
      const data = JSON.parse(event.data) as IConnections;
      const incoming = data.connections ?? [];
      setConnData((old) => {
        const oldConn = old.connections;
        const maxLen = incoming.length;

        const connections: typeof oldConn = [];

        const rest = incoming.filter((each) => {
          const index = oldConn.findIndex((o) => o.id === each.id);

          if (index >= 0 && index < maxLen) {
            const old = oldConn[index];
            each.curUpload = each.upload - old.upload;
            each.curDownload = each.download - old.download;

            connections[index] = each;
            return false;
          }
          return true;
        });

        for (let i = 0; i < maxLen; ++i) {
          if (!connections[i] && rest.length > 0) {
            connections[i] = rest.shift()!;
            connections[i].curUpload = 0;
            connections[i].curDownload = 0;
          }
        }

        return { ...data, connections };
      });
    },
    { errorCount: 3, retryInterval: 1000, onOpen: () => { void syncConnections().catch(() => undefined); } }
  );

  useEffect(() => {
    if (!clashInfo?.server) return;

    const { server = "", secret = "" } = clashInfo;
    connect(`ws://${server}/connections?token=${encodeURIComponent(secret)}`);

    return () => {
      disconnect();
    };
  }, [clashInfo?.server, clashInfo?.secret]);


  useEffect(() => {
    const onRefresh = () => {
      void syncConnections().catch(() => undefined);
    };
    window.addEventListener("verge://connections-refresh", onRefresh);
    return () => window.removeEventListener("verge://connections-refresh", onRefresh);
  }, [syncConnections]);
  const onCloseAll = useLockFn(async () => {
    await closeAllConnections();
    setConnData(initConn);
    await syncConnections().catch(() => undefined);
  });

  const detailRef = useRef<ConnectionDetailRef>(null!);

  return (
    <BasePage
      full
      title={t("Connections")}
      contentStyle={{ height: "100%" }}
      header={
        <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
          <Box sx={{ mx: 1 }}>Download: {parseTraffic(download)}</Box>
          <Box sx={{ mx: 1 }}>Upload: {parseTraffic(upload)}</Box>
          <IconButton
            color="inherit"
            size="small"
            onClick={() =>
              setSetting((o) =>
                o.layout === "list"
                  ? { ...o, layout: "table" }
                  : { ...o, layout: "list" }
              )
            }
          >
            {isTableLayout ? (
              <TableChartRounded fontSize="inherit" />
            ) : (
              <TableRowsRounded fontSize="inherit" />
            )}
          </IconButton>

          <Button size="small" variant="contained" onClick={onCloseAll}>
            {t("Close All")}
          </Button>
        </Box>
      }
    >
      <Box
        sx={{
          pt: 1,
          mb: 0.5,
          mx: "10px",
          height: "36px",
          display: "flex",
          alignItems: "center",
          userSelect: "text",
        }}
      >
        {!isTableLayout && (
          <Select
            size="small"
            autoComplete="off"
            value={curOrderOpt}
            onChange={(e) => setOrderOpt(e.target.value)}
            sx={{
              mr: 1,
              width: i18n.language === "en" ? 190 : 120,
              height: 33.375,
              '[role="button"]': { py: 0.65 },
            }}
          >
            {Object.keys(orderOpts).map((opt) => (
              <MenuItem key={opt} value={opt}>
                <span style={{ fontSize: 14 }}>{t(opt)}</span>
              </MenuItem>
            ))}
          </Select>
        )}

        <TextField
          hiddenLabel
          fullWidth
          size="small"
          autoComplete="off"
          spellCheck="false"
          variant="outlined"
          placeholder={t("Filter conditions")}
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
          sx={{ input: { py: 0.65, px: 1.25 } }}
        />
      </Box>

      <Box height="calc(100% - 50px)" sx={{ userSelect: "text" }}>
        {filterConn.length === 0 ? (
          <BaseEmpty text="No Connections" />
        ) : isTableLayout ? (
          <ConnectionTable
            connections={filterConn}
            onShowDetail={(detail) => detailRef.current?.open(detail)}
          />
        ) : (
          <Virtuoso
            data={filterConn}
            itemContent={(index, item) => (
              <ConnectionItem
                value={item}
                onShowDetail={() => detailRef.current?.open(item)}
              />
            )}
          />
        )}
      </Box>
      <ConnectionDetail ref={detailRef} />
    </BasePage>
  );
};

export default ConnectionsPage;
