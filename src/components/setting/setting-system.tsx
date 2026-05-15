import useSWR from "swr";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton, Tooltip } from "@mui/material";
import { PrivacyTipRounded, Settings, InfoRounded } from "@mui/icons-material";
import { checkService } from "@/services/cmds";
import { useVerge } from "@/hooks/use-verge";
import { DialogRef, Notice, Switch } from "@/components/base";
import { SettingList, SettingItem } from "./mods/setting-comp";
import { GuardState } from "./mods/guard-state";
import { ServiceViewer } from "./mods/service-viewer";
import { SysproxyViewer } from "./mods/sysproxy-viewer";
import { TunViewer } from "./mods/tun-viewer";
import getSystem from "@/utils/get-system";

interface Props {
  onError?: (err: Error) => void;
}

const isWIN = getSystem() === "windows";
const SWITCH_OPERATION_IN_PROGRESS =
  "Another switch operation is already in progress";
const isSwitchOperationInProgressError = (err: unknown) =>
  err instanceof Error && err.message === SWITCH_OPERATION_IN_PROGRESS;

const SettingSystem = ({ onError }: Props) => {
  const { t } = useTranslation();

  const { verge, mutateVerge, patchVerge } = useVerge();

  // service mode
  const { data: serviceStatus, mutate: mutateServiceStatus } = useSWR(
    isWIN ? "checkService" : null,
    checkService,
    {
      revalidateIfStale: false,
      shouldRetryOnError: false,
      focusThrottleInterval: 5000, // 5s
    }
  );

  const serviceRef = useRef<DialogRef>(null);
  const sysproxyRef = useRef<DialogRef>(null);
  const tunRef = useRef<DialogRef>(null);

  const {
    enable_tun_mode,
    enable_auto_launch,
    enable_service_mode,
    enable_silent_start,
    enable_system_proxy,
  } = verge ?? {};

  const onSwitchFormat = (_e: any, value: boolean) => value;
  const [pendingSwitch, setPendingSwitch] = useState<
    "tun" | "service" | "sysproxy" | null
  >(null);
  const switchesBusy = pendingSwitch !== null;
  const onChangeData = (patch: Partial<IVergeConfig>) => {
    mutateVerge({ ...verge, ...patch }, false);
  };
  const onSwitchCatch = (err: Error) => {
    if (isSwitchOperationInProgressError(err)) return;
    onError?.(err);
  };

  const serviceChecking = isWIN && serviceStatus == null;
  const serviceInstalled = !!serviceStatus?.installed;
  const serviceReady =
    !!serviceStatus?.installed && !!serviceStatus?.running && !!serviceStatus?.api_ready;
  const serviceSwitchDisabled = isWIN && (serviceChecking || !serviceInstalled);

  return (
    <SettingList title={t("System Setting")}>
      <SysproxyViewer ref={sysproxyRef} />
      <TunViewer ref={tunRef} />
      {isWIN && (
        <ServiceViewer
          ref={serviceRef}
          enable={!!enable_service_mode}
          enableTun={!!enable_tun_mode}
          onStatusChange={async () => {
            await mutateServiceStatus();
            await mutateVerge();
          }}
        />
      )}

      <SettingItem
        label={t("Tun Mode")}
        extra={
          <>
            <Tooltip
              title={
                isWIN ? t("Tun Mode Info Windows") : t("Tun Mode Info Unix")
              }
              placement="top"
            >
              <IconButton color="inherit" size="small">
                <InfoRounded
                  fontSize="inherit"
                  style={{ cursor: "pointer", opacity: 0.75 }}
                />
              </IconButton>
            </Tooltip>
            <IconButton
              color="inherit"
              size="small"
              onClick={() => tunRef.current?.open()}
            >
              <Settings
                fontSize="inherit"
                style={{ cursor: "pointer", opacity: 0.75 }}
              />
            </IconButton>
          </>
        }
      >
        <GuardState
          value={enable_tun_mode ?? false}
          valueProps="checked"
          onCatch={onSwitchCatch}
          onFormat={onSwitchFormat}
            onChange={(e) => onChangeData({ enable_tun_mode: e })}
            onGuard={async (e) => {
              if (pendingSwitch !== null) {
                throw new Error(SWITCH_OPERATION_IN_PROGRESS);
              }
              setPendingSwitch("tun");
              try {
                if (isWIN && e) {
                  const latestServiceStatus = await mutateServiceStatus();
                  if (!latestServiceStatus?.installed) {
                    throw new Error("Please install and enable Service Mode first.");
                  }
                  if (!enable_service_mode) {
                    throw new Error("Please enable Service Mode first.");
                  }
                  if (!serviceReady) {
                    Notice.info("Checking service readiness...");
                  }
                }
                await patchVerge({ enable_tun_mode: e });
                await mutateVerge();
                if (isWIN) await mutateServiceStatus();
              } catch (err) {
                await mutateVerge();
                if (isWIN) await mutateServiceStatus();
                throw err;
              } finally {
                setPendingSwitch(null);
              }
            }}
        >
          <Switch edge="end" disabled={switchesBusy} />
        </GuardState>
      </SettingItem>

      {isWIN && (
        <SettingItem
          label={t("Service Mode")}
          extra={
            <IconButton
              color="inherit"
              size="small"
              onClick={() => serviceRef.current?.open()}
            >
              <PrivacyTipRounded
                fontSize="inherit"
                style={{ cursor: "pointer", opacity: 0.75 }}
              />
            </IconButton>
          }
        >
          <GuardState
            value={enable_service_mode ?? false}
            valueProps="checked"
            onCatch={onSwitchCatch}
            onFormat={onSwitchFormat}
            onChange={(e) => onChangeData({ enable_service_mode: e })}
            onGuard={async (e) => {
              if (pendingSwitch !== null) {
                throw new Error(SWITCH_OPERATION_IN_PROGRESS);
              }
              setPendingSwitch("service");
              try {
                if (isWIN) {
                  const latestServiceStatus = await mutateServiceStatus();
                  if (enable_tun_mode) {
                    throw new Error(
                      "Tun Mode is enabled. Please disable Tun Mode before changing Service Mode."
                    );
                  }
                  if (e && !latestServiceStatus?.installed) {
                    throw new Error("Please install Service first from the shield button.");
                  }
                }
                await patchVerge({ enable_service_mode: e });
                await mutateVerge();
                if (isWIN) await mutateServiceStatus();
              } catch (err) {
                await mutateVerge();
                if (isWIN) await mutateServiceStatus();
                throw err;
              } finally {
                setPendingSwitch(null);
              }
            }}
          >
            <Switch
              edge="end"
              disabled={switchesBusy || !!enable_tun_mode || serviceSwitchDisabled}
            />
          </GuardState>
        </SettingItem>
      )}

      <SettingItem
        label={t("System Proxy")}
        extra={
          <IconButton
            color="inherit"
            size="small"
            onClick={() => sysproxyRef.current?.open()}
          >
            <Settings
              fontSize="inherit"
              style={{ cursor: "pointer", opacity: 0.75 }}
            />
          </IconButton>
        }
      >
        <GuardState
          value={enable_system_proxy ?? false}
          valueProps="checked"
          onCatch={onSwitchCatch}
          onFormat={onSwitchFormat}
          onChange={(e) => onChangeData({ enable_system_proxy: e })}
          onGuard={async (e) => {
            if (pendingSwitch !== null) {
              throw new Error(SWITCH_OPERATION_IN_PROGRESS);
            }
            setPendingSwitch("sysproxy");
            try {
              await patchVerge({ enable_system_proxy: e });
              await mutateVerge();
            } catch (err) {
              await mutateVerge();
              if (isWIN) await mutateServiceStatus();
              throw err;
            } finally {
              setPendingSwitch(null);
            }
          }}
        >
          <Switch edge="end" disabled={switchesBusy} />
        </GuardState>
      </SettingItem>

      <SettingItem label={t("Auto Launch")}>
        <GuardState
          value={enable_auto_launch ?? false}
          valueProps="checked"
          onCatch={onError}
          onFormat={onSwitchFormat}
          onChange={(e) => onChangeData({ enable_auto_launch: e })}
          onGuard={(e) => patchVerge({ enable_auto_launch: e })}
        >
          <Switch edge="end" />
        </GuardState>
      </SettingItem>

      <SettingItem label={t("Silent Start")}>
        <GuardState
          value={enable_silent_start ?? false}
          valueProps="checked"
          onCatch={onError}
          onFormat={onSwitchFormat}
          onChange={(e) => onChangeData({ enable_silent_start: e })}
          onGuard={(e) => patchVerge({ enable_silent_start: e })}
        >
          <Switch edge="end" />
        </GuardState>
      </SettingItem>
    </SettingList>
  );
};

export default SettingSystem;
