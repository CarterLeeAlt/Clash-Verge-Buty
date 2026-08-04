import { mutate } from "swr";
import { forwardRef, useImperativeHandle, useState } from "react";
import { BaseDialog, DialogRef, formatNoticeMessage, Notice } from "@/components/base";
import { useTranslation } from "react-i18next";
import { useVerge } from "@/hooks/use-verge";
import { useLockFn } from "ahooks";
import { LoadingButton } from "@mui/lab";
import { SwitchAccessShortcut, RestartAlt } from "@mui/icons-material";
import {
  Box,
  Button,
  Tooltip,
  List,
  ListItemButton,
  ListItemText,
} from "@mui/material";
import { changeClashCore, restartSidecar } from "@/services/cmds";
import { closeAllConnections, upgradeCore } from "@/services/api";
import { grantPermission } from "@/services/cmds";
import getSystem from "@/utils/get-system";

const VALID_CORE = [
  { name: "Mihomo", core: "mihomo" },
  { name: "Mihomo Alpha", core: "mihomo-alpha" },
];

const OS = getSystem();

export const ClashCoreViewer = forwardRef<DialogRef>((props, ref) => {
  const { t } = useTranslation();

  const { verge, mutateVerge } = useVerge();

  const [open, setOpen] = useState(false);
  const [upgrading, setUpgrading] = useState(false);

  useImperativeHandle(ref, () => ({
    open: () => setOpen(true),
    close: () => setOpen(false),
  }));

  const { clash_core = "mihomo" } = verge ?? {};

  const onCoreChange = useLockFn(async (core: string) => {
    if (core === clash_core) return;

    try {
      closeAllConnections();
      await changeClashCore(core);
      mutateVerge();
      setTimeout(() => {
        mutate("getClashConfig");
        mutate("getVersion");
      }, 100);
      Notice.success(`Switched to ${core}.`);
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  const onGrant = useLockFn(async (core: string) => {
    try {
      await grantPermission(core);
      // 自动重启
      if (core === clash_core) await restartSidecar();
      Notice.success(`Permission granted for ${core}.`);
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  const onRestart = useLockFn(async () => {
    try {
      await restartSidecar();
      Notice.success(`Mihomo core restarted.`);
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  const onUpgrade = useLockFn(async () => {
    try {
      setUpgrading(true);
      await upgradeCore();
      setUpgrading(false);
      Notice.success(`Mihomo core upgraded.`);
    } catch (err: any) {
      setUpgrading(false);
      Notice.error(formatNoticeMessage(err?.response?.data?.message || err));
    }
  });

  return (
    <BaseDialog
      open={open}
      title={
        <Box display="flex" justifyContent="space-between">
          {t("Mihomo Core")}
          <Box>
            {clash_core !== "mihomo" && (
              <LoadingButton
                variant="contained"
                size="small"
                startIcon={<SwitchAccessShortcut />}
                loadingPosition="start"
                loading={upgrading}
                sx={{ marginRight: "8px" }}
                onClick={onUpgrade}
              >
                {t("Upgrade")}
              </LoadingButton>
            )}
            <Button
              variant="contained"
              size="small"
              onClick={onRestart}
              startIcon={<RestartAlt />}
            >
              {t("Restart")}
            </Button>
          </Box>
        </Box>
      }
      contentSx={{
        pb: 0,
        width: 400,
        height: 180,
        overflowY: "auto",
        userSelect: "text",
        marginTop: "-8px",
      }}
      disableOk
      cancelBtn={t("Back")}
      onClose={() => setOpen(false)}
      onCancel={() => setOpen(false)}
    >
      <List component="nav">
        {VALID_CORE.map((each) => (
          <ListItemButton
            key={each.core}
            selected={each.core === clash_core}
            onClick={() => onCoreChange(each.core)}
          >
            <ListItemText primary={each.name} secondary={`/${each.core}`} />

            {(OS === "macos" || OS === "linux") && (
              <Tooltip title={t("Tun mode requires")}>
                <Button
                  variant="outlined"
                  size="small"
                  onClick={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    onGrant(each.core);
                  }}
                >
                  {t("Grant")}
                </Button>
              </Tooltip>
            )}
          </ListItemButton>
        ))}
      </List>
    </BaseDialog>
  );
});
