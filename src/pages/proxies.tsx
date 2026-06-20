import useSWR from "swr";
import { useEffect, useMemo } from "react";
import { useLockFn } from "ahooks";
import { useTranslation } from "react-i18next";
import lockIconUrl from "@/assets/icons/lock.svg";
import lockOpenIconUrl from "@/assets/icons/lock_open.svg";
import restartIconUrl from "@/assets/icons/restart.svg";
import { Box, Button, ButtonGroup, IconButton, Tooltip } from "@mui/material";
import {
  closeAllConnections,
  getClashConfig,
  updateConfigs,
} from "@/services/api";
import {
  patchClashConfig,
  restartSidecar,
  setWindowSizeLocked,
} from "@/services/cmds";
import { useVerge } from "@/hooks/use-verge";
import { BasePage, formatNoticeMessage, Notice } from "@/components/base";
import { ProxyGroups } from "@/components/proxy/proxy-groups";
import { ProviderButton } from "@/components/proxy/provider-button";

function LocalSvgIcon(props: { src: string; size?: number }) {
  const { src, size = 22 } = props;

  return (
    <img
      aria-hidden
      src={src}
      draggable={false}
      style={{
        display: "block",
        width: size,
        height: size,
        objectFit: "contain",
        pointerEvents: "none",
        userSelect: "none",
      }}
    />
  );
}

const ProxyPage = () => {
  const { t } = useTranslation();

  const { data: clashConfig, mutate: mutateClash } = useSWR(
    "getClashConfig",
    getClashConfig
  );

  const { verge, mutateVerge } = useVerge();

  const isSizeLocked = verge?.window_size_locked ?? false;

  const modeList = useMemo(() => {
    if (verge?.clash_core?.includes("clash-meta")) {
      return ["rule", "global", "direct"];
    }
    return ["rule", "global", "direct", "script"];
  }, [verge?.clash_core]);

  const curMode = clashConfig?.mode?.toLowerCase();

  const onChangeMode = useLockFn(async (mode: string) => {
    // 断开连接
    if (mode !== curMode && verge?.auto_close_connection) {
      closeAllConnections();
    }
    await updateConfigs({ mode });
    await patchClashConfig({ mode });
    mutateClash();
  });

  const onRestartCore = useLockFn(async () => {
    try {
      await restartSidecar();
      Notice.success(`Clash core restarted.`);
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  const onToggleSizeLocked = useLockFn(async () => {
    try {
      await setWindowSizeLocked(!isSizeLocked);
      await mutateVerge();
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  useEffect(() => {
    if (curMode && !modeList.includes(curMode)) {
      onChangeMode("rule");
    }
  }, [curMode]);

  return (
    <BasePage
      full
      contentStyle={{ height: "100%" }}
      title={t("Proxy Groups")}
      header={
        <Box display="flex" alignItems="center" gap={1}>
          <ProviderButton />

          <Box display="flex" alignItems="center">
            <Tooltip
              title={
                isSizeLocked ? t("Unlock Window Size") : t("Lock Window Size")
              }
            >
              <IconButton
                size="small"
                color="inherit"
                onClick={onToggleSizeLocked}
                sx={{ mr: 1 }}
              >
                <LocalSvgIcon
                  src={isSizeLocked ? lockIconUrl : lockOpenIconUrl}
                />
              </IconButton>
            </Tooltip>

            <Tooltip title={t("Restart")}>
              <IconButton
                size="small"
                color="inherit"
                onClick={onRestartCore}
                sx={{ mr: 1.2 }}
              >
                <LocalSvgIcon src={restartIconUrl} />
              </IconButton>
            </Tooltip>

            <ButtonGroup size="small">
              {modeList.map((mode) => (
                <Button
                  key={mode}
                  variant={mode === curMode ? "contained" : "outlined"}
                  onClick={() => onChangeMode(mode)}
                  sx={{ textTransform: "capitalize" }}
                >
                  {t(mode)}
                </Button>
              ))}
            </ButtonGroup>
          </Box>
        </Box>
      }
    >
      <ProxyGroups mode={curMode!} />
    </BasePage>
  );
};

export default ProxyPage;
