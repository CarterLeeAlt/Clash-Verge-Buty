import useSWR from "swr";
import { forwardRef, useImperativeHandle, useState } from "react";
import { useLockFn } from "ahooks";
import { useTranslation } from "react-i18next";
import { Button, Stack, Typography } from "@mui/material";
import {
  checkService,
  installService,
  uninstallService,
  patchVergeConfig,
} from "@/services/cmds";
import { BaseDialog, DialogRef, Notice } from "@/components/base";

interface Props {
  enable: boolean;
  enableTun: boolean;
  onStatusChange?: () => Promise<void> | void;
}

export const ServiceViewer = forwardRef<DialogRef, Props>((props, ref) => {
  const { enable, enableTun, onStatusChange } = props;

  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  const { data: status, mutate: mutateCheck } = useSWR(
    "checkService",
    checkService,
    {
      revalidateIfStale: false,
      shouldRetryOnError: false,
      focusThrottleInterval: 5000, // 5s
    }
  );

  useImperativeHandle(ref, () => ({
    open: () => setOpen(true),
    close: () => setOpen(false),
  }));

  const state =
    status == null
      ? "pending"
      : !status.installed
        ? "service_not_installed"
        : !status.running
          ? "service_installed_stopped"
          : !status.api_ready
            ? "service_running_api_not_ready"
            : !status.core_managed
              ? "service_running_api_ready_core_not_managed"
              : "service_running_api_ready_core_managed";

  const onInstall = useLockFn(async () => {
    try {
      await installService();
      await mutateCheck();
      await onStatusChange?.();
      setOpen(false);
      Notice.success("Service installed successfully. You can now enable Service Mode.");
    } catch (err: any) {
      mutateCheck();
      Notice.error(err.message || err.toString());
    }
  });

  const onUninstall = useLockFn(async () => {
    try {
      if (enableTun) {
        throw new Error(
          "Tun Mode is enabled. Please disable Tun Mode before uninstalling the service."
        );
      }
      if (enable || enableTun) {
        await patchVergeConfig({
          enable_service_mode: false,
          enable_tun_mode: false,
        } as IVergeConfig);
      }

      await uninstallService();
      await mutateCheck();
      await onStatusChange?.();
      setOpen(false);
      Notice.success("Service uninstalled successfully");
    } catch (err: any) {
      mutateCheck();
      Notice.error(err.message || err.toString());
    }
  });

  // fix unhandled error of the service mode
  const onDisable = useLockFn(async () => {
    try {
      await patchVergeConfig({
        enable_service_mode: false,
        enable_tun_mode: false,
      } as IVergeConfig);
      await mutateCheck();
      await onStatusChange?.();
      setOpen(false);
    } catch (err: any) {
      mutateCheck();
      Notice.error(err.message || err.toString());
    }
  });

  return (
    <BaseDialog
      open={open}
      title={t("Service Mode")}
      contentSx={{ width: 360, userSelect: "text" }}
      disableFooter
      onClose={() => setOpen(false)}
    >
      <Typography>
        Current State:{" "}
        {state.startsWith("service_running") ? "running" : state}
      </Typography>
      <Typography>Information: {status?.message}</Typography>

      <Stack
        direction="row"
        spacing={1}
        sx={{ mt: 4, justifyContent: "flex-end" }}
      >
        {state === "service_not_installed" && enable && (
          <Button variant="contained" onClick={onDisable}>
            Disable Service Mode
          </Button>
        )}

        {state === "service_not_installed" && (
          <Button variant="contained" onClick={onInstall}>
            Install
          </Button>
        )}

        {state !== "service_not_installed" && (
          <Button
            variant="outlined"
            onClick={onUninstall}
            disabled={enableTun}
          >
            Uninstall
          </Button>
        )}
      </Stack>
    </BaseDialog>
  );
});
