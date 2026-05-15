import { useCallback, useEffect, useRef } from "react";

export type WsMsgFn = (event: MessageEvent<any>) => void;

export interface WsOptions {
  errorCount?: number; // legacy alias of maxRetries
  retryInterval?: number; // default is 2500
  maxRetryInterval?: number;
  maxRetries?: number | "infinite";
  reconnect?: boolean;
  backoff?: boolean;
  onError?: () => void;
  onOpen?: () => void;
  onClose?: () => void;
  shouldReconnect?: boolean; // default is true
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

    const scheduleReconnect = () => {
      const opts = optsRef.current;
      const reconnectEnabled = (opts?.reconnect ?? opts?.shouldReconnect ?? true) !== false;

      if (!reconnectEnabled || manualCloseRef.current || unmountedRef.current) return;
      if (wsRef.current !== ws || timerRef.current || !urlRef.current) return;

      const maxRetries = opts?.maxRetries ?? opts?.errorCount ?? 5;
      if (maxRetries !== "infinite" && retryCountRef.current >= maxRetries) {
        opts?.onError?.();
        return;
      }

      const retryInterval = opts?.retryInterval ?? 2500;
      const maxRetryInterval = opts?.maxRetryInterval ?? retryInterval;
      const backoff = opts?.backoff ?? false;
      const delay = backoff
        ? Math.min(retryInterval * 2 ** retryCountRef.current, maxRetryInterval)
        : retryInterval;

      retryCountRef.current += 1;
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        if (manualCloseRef.current || unmountedRef.current) return;
        connect(urlRef.current, false);
      }, delay);
    };

    ws.onopen = () => {
      if (wsRef.current !== ws) return;
      retryCountRef.current = 0;
      optsRef.current?.onOpen?.();
    };

    ws.onmessage = (event) => {
      if (wsRef.current !== ws) return;
      msgRef.current(event);
    };

    ws.onerror = () => {
      if (wsRef.current !== ws) return;
      optsRef.current?.onError?.();
      scheduleReconnect();
    };

    ws.onclose = () => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      optsRef.current?.onClose?.();
      scheduleReconnect();
    };
  }, [clearTimer, closeWs]);

  const disconnect = useCallback(() => {
    manualCloseRef.current = true;
    clearTimer();
    closeWs();
  }, [clearTimer, closeWs]);

  useEffect(() => () => {
    unmountedRef.current = true;
    disconnect();
  }, [disconnect]);

  return { connect, disconnect };
};
