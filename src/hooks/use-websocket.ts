import { useCallback, useEffect, useRef } from "react";

export type WsMsgFn = (event: MessageEvent<any>) => void;

export interface WsOptions {
  errorCount?: number; // legacy alias of maxRetries
  retryInterval?: number; // default is 2500
  maxRetryInterval?: number;
  maxRetries?: number | "infinite";
  reconnect?: boolean;
  backoff?: boolean;
  onError?: (event: Event) => void;
  onOpen?: (event: Event) => void;
  onClose?: (event: CloseEvent) => void;
  shouldReconnect?: boolean; // legacy alias of reconnect
}

export const useWebsocket = (onMessage: WsMsgFn, options?: WsOptions) => {
  const wsRef = useRef<WebSocket | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryCountRef = useRef(0);
  const manualCloseRef = useRef(false);
  const unmountedRef = useRef(false);
  const urlRef = useRef("");
  const msgRef = useRef(onMessage);
  const optsRef = useRef(options);
  const connIdRef = useRef(0);

  msgRef.current = onMessage;
  optsRef.current = options;

  const clearTimer = useCallback(() => {
    if (!timerRef.current) return;
    clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const closeWs = useCallback((ws?: WebSocket | null) => {
    const target = ws ?? wsRef.current;
    if (!target) return;

    target.onopen = null;
    target.onmessage = null;
    target.onerror = null;
    target.onclose = null;

    try {
      target.close();
    } catch {
      // ignore close failures
    }

    if (wsRef.current === target) wsRef.current = null;
  }, []);

  const connect = useCallback((url: string, resetRetry = true) => {
    if (!url || unmountedRef.current) return;

    const prevUrl = urlRef.current;
    manualCloseRef.current = false;
    urlRef.current = url;

    if (resetRetry || prevUrl !== url) retryCountRef.current = 0;

    clearTimer();
    closeWs(wsRef.current);

    const ws = new WebSocket(url);
    wsRef.current = ws;
    const connId = connIdRef.current + 1;
    connIdRef.current = connId;

    const scheduleReconnect = () => {
      const opts = optsRef.current;
      const reconnectEnabled = (opts?.reconnect ?? opts?.shouldReconnect ?? true) !== false;
      if (!reconnectEnabled || manualCloseRef.current || unmountedRef.current) return;
      if (connIdRef.current !== connId || timerRef.current || !urlRef.current) return;

      const maxRetries = opts?.maxRetries ?? opts?.errorCount ?? 5;
      const normalizedMaxRetries =
        maxRetries === "infinite"
          ? "infinite"
          : Number.isFinite(maxRetries)
            ? Math.max(0, Math.floor(maxRetries))
            : 5;
      if (normalizedMaxRetries !== "infinite" && retryCountRef.current >= normalizedMaxRetries) return;

      const retryInterval = Number.isFinite(opts?.retryInterval)
        ? Math.max(0, opts!.retryInterval as number)
        : 2500;
      const maxRetryInterval = Number.isFinite(opts?.maxRetryInterval)
        ? Math.max(retryInterval, opts!.maxRetryInterval as number)
        : retryInterval;
      const backoff = opts?.backoff ?? false;
      const delay = backoff
        ? Math.min(retryInterval * 2 ** retryCountRef.current, maxRetryInterval)
        : retryInterval;

      retryCountRef.current += 1;
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        if (manualCloseRef.current || unmountedRef.current || connIdRef.current !== connId) return;
        connect(urlRef.current, false);
      }, delay);
    };

    ws.onopen = (event) => {
      if (wsRef.current !== ws || connIdRef.current !== connId) return;
      retryCountRef.current = 0;
      clearTimer();
      optsRef.current?.onOpen?.(event);
    };

    ws.onmessage = (event) => {
      if (wsRef.current !== ws || connIdRef.current !== connId) return;
      msgRef.current(event);
    };

    ws.onerror = (event) => {
      if (connIdRef.current !== connId) return;
      optsRef.current?.onError?.(event);
      scheduleReconnect();
    };

    ws.onclose = (event) => {
      if (connIdRef.current !== connId) return;
      if (wsRef.current === ws) wsRef.current = null;
      optsRef.current?.onClose?.(event);
      scheduleReconnect();
    };
  }, [clearTimer, closeWs]);

  const disconnect = useCallback(() => {
    manualCloseRef.current = true;
    retryCountRef.current = 0;
    connIdRef.current += 1;
    clearTimer();
    closeWs();
  }, [clearTimer, closeWs]);

  useEffect(() => () => {
    unmountedRef.current = true;
    disconnect();
  }, [disconnect]);

  return { connect, disconnect };
};
