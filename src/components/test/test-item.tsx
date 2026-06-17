import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { useLockFn } from "ahooks";
import { useTranslation } from "react-i18next";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  Box,
  Typography,
  Divider,
  MenuItem,
  Menu,
  styled,
  alpha,
} from "@mui/material";
import { BaseLoading } from "@/components/base";
import { LanguageTwoTone } from "@mui/icons-material";
import { formatNoticeMessage, Notice } from "@/components/base";
import { TestBox } from "./test-box";
import delayManager from "@/services/delay";
import { cmdTestDelay } from "@/services/cmds";
import { listen } from "@tauri-apps/api/event";

interface Props {
  id: string;
  itemData: IVergeTestItem;
  editable?: boolean;
  onEdit: () => void;
  onDelete: (uid: string) => void;
}

export const TestItem = (props: Props) => {
  const { itemData, editable = true, onEdit, onDelete: onDeleteItem } = props;
  const { attributes, listeners, setNodeRef, transform, transition } =
    useSortable({ id: props.id });

  const { t } = useTranslation();
  const [anchorEl, setAnchorEl] = useState<any>(null);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const [delay, setDelay] = useState(-1);
  const [proxy, setProxy] = useState<string>();
  const [isProxyOverflowing, setIsProxyOverflowing] = useState(false);
  const [proxyScrollDistance, setProxyScrollDistance] = useState(0);
  const proxyContainerRef = useRef<HTMLSpanElement>(null);
  const proxyTextRef = useRef<HTMLSpanElement>(null);
  const [iconLoadFailed, setIconLoadFailed] = useState(false);
  const { uid, name, icon, url } = itemData;

  const onDelay = useCallback(async () => {
    setDelay(-2);
    setProxy(undefined);
    const result = await cmdTestDelay(url);
    setDelay(result.delay);
    setProxy(result.proxy);
  }, [url]);

  const onEditTest = () => {
    setAnchorEl(null);

    if (!editable) {
      return;
    }

    onEdit();
  };

  const onDelete = useLockFn(async () => {
    setAnchorEl(null);

    if (!editable) {
      return;
    }

    try {
      onDeleteItem(uid);
    } catch (err: any) {
      Notice.error(formatNoticeMessage(err));
    }
  });

  const menu = [
    { label: "Edit", handler: onEditTest },
    { label: "Delete", handler: onDelete },
  ];

  useEffect(() => {
    const proxyContainer = proxyContainerRef.current;
    const proxyText = proxyTextRef.current;

    if (!proxyContainer || !proxyText) {
      setIsProxyOverflowing(false);
      return;
    }

    const updateOverflow = () => {
      const scrollDistance = proxyText.scrollWidth - proxyContainer.clientWidth;
      setIsProxyOverflowing(scrollDistance > 0);
      setProxyScrollDistance(Math.max(scrollDistance, 0));
    };

    updateOverflow();

    const resizeObserver = new ResizeObserver(updateOverflow);
    resizeObserver.observe(proxyContainer);
    resizeObserver.observe(proxyText);
    window.addEventListener("resize", updateOverflow);

    return () => {
      resizeObserver.disconnect();
      window.removeEventListener("resize", updateOverflow);
    };
  }, [proxy, delay]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    listen("verge://test-all", () => {
      onDelay();
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [onDelay]);

  return (
    <Box
      sx={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      <TestBox
        onClick={onEditTest}
        onDoubleClick={onEditTest}
        onContextMenu={(event) => {
          event.preventDefault();

          if (!editable) {
            setAnchorEl(null);
            return;
          }

          const { clientX, clientY } = event;
          setPosition({ top: clientY, left: clientX });
          setAnchorEl(event.currentTarget);
        }}
      >
        <Box
          position="relative"
          sx={{ cursor: "move" }}
          ref={setNodeRef}
          {...attributes}
          {...listeners}
        >
          {icon && icon.trim() !== "" && !iconLoadFailed ? (
            <Box
              sx={{
                display: "flex",
                justifyContent: "center",
                minHeight: "40px",
              }}
            >
              <img
                src={
                  icon.trim().startsWith("<svg")
                    ? `data:image/svg+xml;base64,${btoa(icon)}`
                    : icon
                }
                alt={`${name} icon`}
                height="40"
                width="40"
                onError={() => setIconLoadFailed(true)}
                style={{ objectFit: "contain" }}
              />
            </Box>
          ) : (
            <Box
              sx={{
                display: "flex",
                justifyContent: "center",
                minHeight: "40px",
              }}
            >
              <LanguageTwoTone sx={{ height: "40px" }} fontSize="large" />
            </Box>
          )}

          <Box sx={{ display: "flex", justifyContent: "center" }}>
            <Typography
              variant="h6"
              component="h2"
              noWrap
              title={name}
              sx={{ fontSize: 16 }}
            >
              {name}
            </Typography>
          </Box>
        </Box>
        <Divider sx={{ marginTop: "8px" }} />
        <Box
          sx={{
            display: "flex",
            justifyContent: "center",
            marginTop: "8px",
            color: "primary.main",
          }}
        >
          {delay === -2 && (
            <Widget>
              <BaseLoading />
            </Widget>
          )}

          {delay === -1 && (
            <Widget
              className="the-check"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                onDelay();
              }}
              sx={({ palette }) => ({
                ":hover": { bgcolor: alpha(palette.primary.main, 0.15) },
              })}
            >
              Check
            </Widget>
          )}

          {delay >= 0 && (
            // 显示延迟
            <Widget
              className="the-delay"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                onDelay();
              }}
              color={delayManager.formatDelayColor(delay)}
              sx={({ palette }) => ({
                ":hover": {
                  bgcolor: alpha(palette.primary.main, 0.15),
                },
              })}
            >
              <DelayText>{delayManager.formatDelay(delay)}</DelayText>
              <DelaySeparator>|</DelaySeparator>
              <ProxyName ref={proxyContainerRef}>
                <ProxyNameText
                  ref={proxyTextRef}
                  className={isProxyOverflowing ? "scrolling" : undefined}
                  style={
                    {
                      "--proxy-scroll-distance": `-${proxyScrollDistance}px`,
                    } as CSSProperties
                  }
                  title={proxy || t("Unknown")}
                >
                  {proxy || t("Unknown")}
                </ProxyNameText>
              </ProxyName>
            </Widget>
          )}
        </Box>
      </TestBox>

      {editable && (
        <Menu
          open={!!anchorEl}
          anchorEl={anchorEl}
          onClose={() => setAnchorEl(null)}
          anchorPosition={position}
          anchorReference="anchorPosition"
          transitionDuration={225}
          MenuListProps={{ sx: { py: 0.5 } }}
          onContextMenu={(e) => {
            setAnchorEl(null);
            e.preventDefault();
          }}
        >
          {menu.map((item) => (
            <MenuItem
              key={item.label}
              onClick={item.handler}
              sx={{ minWidth: 120 }}
              dense
            >
              {t(item.label)}
            </MenuItem>
          ))}
        </Menu>
      )}
    </Box>
  );
};
const Widget = styled(Box)(({ theme: { typography } }) => ({
  padding: "3px 6px",
  fontSize: 14,
  fontFamily: typography.fontFamily,
  borderRadius: "4px",
  maxWidth: "100%",
  boxSizing: "border-box",
  "&.the-delay": {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    minWidth: 0,
  },
}));

const DelayText = styled("span")({
  flex: "0 0 auto",
  whiteSpace: "nowrap",
});

const DelaySeparator = styled("span")({
  flex: "0 0 auto",
  margin: "0 4px",
});

const ProxyName = styled("span")({
  flex: "1 1 auto",
  minWidth: 0,
  overflow: "hidden",
  whiteSpace: "nowrap",
});

const ProxyNameText = styled("span")({
  display: "inline-block",
  whiteSpace: "nowrap",
  "&.scrolling": {
    animation: "proxy-name-scroll 8s linear infinite",
  },
  "@keyframes proxy-name-scroll": {
    "0%, 15%": {
      transform: "translateX(0)",
    },
    "85%, 100%": {
      transform: "translateX(var(--proxy-scroll-distance))",
    },
  },
});
