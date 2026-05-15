import useSWR from "swr";
import { forwardRef, useEffect, useImperativeHandle, useState } from "react";
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
  const [operation, setOperation] = useState<"install" | "uninstall" | null>(
    null
  );
  const [showPendingHint, setShowPendingHint] = useState(false);
  const isPending = operation !== null;

  useEffect(() => {
    if (!isPending) {
      setShowPendingHint(false);
      return;
    }

    const timer = setTimeout(() => {
      setShowPendingHint(true);
    }, 2000);

    return () => clearTimeout(timer);
  }, [isPending]);

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
    setOperation("install");
    try {
      await installService();
      setOpen(false);
      void Promise.allSettled([mutateCheck(), Promise.resolve(onStatusChange?.())])
        .then((results) => {
          results.forEach((result) => {
            if (result.status === "rejected") {
              console.warn("service status refresh failed:", result.reason);
            }
          });
        })
        .catch((err) => console.warn("service status refresh failed:", err));
      Notice.success("Service installed successfully. You can now enable Service Mode.");
    } catch (err: any) {
      mutateCheck();
      Notice.error(err.message || err.toString());
    } finally {
      setOperation(null);
    }
  });

  const onUninstall = useLockFn(async () => {
    setOperation("uninstall");
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
      setOpen(false);
      void Promise.allSettled([mutateCheck(), Promise.resolve(onStatusChange?.())])
        .then((results) => {
          results.forEach((result) => {
            if (result.status === "rejected") {
              console.warn("service status refresh failed:", result.reason);
            }
          });
        })
        .catch((err) => console.warn("service status refresh failed:", err));
      Notice.success("Service uninstalled successfully");
    } catch (err: any) {
      mutateCheck();
      Notice.error(err.message || err.toString());
    } finally {
      setOperation(null);
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
      disableEscapeKeyDown={isPending}
      disableBackdropClose={isPending}
      onClose={() => !isPending && setOpen(false)}
    >
      <Typography>
        Current State:{" "}
        {state.startsWith("service_running") ? "running" : state}
      </Typography>
      <Typography>Information: {status?.message}</Typography>
      {showPendingHint && (
        <Typography sx={{ mt: 1 }}>
          Windows is stopping and removing the service, please wait...
        </Typography>
      )}

      <Stack
        direction="row"
        spacing={1}
        sx={{ mt: 4, justifyContent: "flex-end" }}
      >
        {state === "service_not_installed" && enable && (
          <Button variant="contained" onClick={onDisable} disabled={isPending}>
            Disable Service Mode
          </Button>
        )}

        {state === "service_not_installed" && (
          <Button variant="contained" onClick={onInstall} disabled={isPending}>
            {operation === "install" ? "Installing..." : "Install"}
          </Button>
        )}

        {state !== "service_not_installed" && (
          <Button
            variant="outlined"
            onClick={onUninstall}
            disabled={enableTun || isPending}
          >
            {operation === "uninstall" ? "Uninstalling..." : "Uninstall"}
          </Button>
        )}
      </Stack>
    </BaseDialog>
  );
});
