import { useCallback, useEffect, useRef } from "react";

export type WsMsgFn = (event: MessageEvent<any>) => void;

export interface WsOptions {
  errorCount?: number; // default is 5
  retryInterval?: number; // default is 2500
  onError?: () => void;
  onOpen?: () => void;
  onClose?: () => void;
  shouldReconnect?: boolean; // default is true
}

export const useWebsocket = (onMessage: WsMsgFn, options?: WsOptions) => {
  const wsRef = useRef<WebSocket | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryLeftRef = useRef(options?.errorCount ?? 5);
  const manualCloseRef = useRef(false);
  const unmountedRef = useRef(false);
  const urlRef = useRef("");
  const msgRef = useRef(onMessage);
  const optsRef = useRef(options);

  msgRef.current = onMessage;
  optsRef.current = options;

  const clearTimer = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
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
    } catch {}

    if (wsRef.current === target) {
      wsRef.current = null;
    }
  }, []);

  const connect = useCallback((url: string, resetRetry = true) => {
    if (!url || unmountedRef.current) return;

    const prevUrl = urlRef.current;
    manualCloseRef.current = false;
    urlRef.current = url;

    if (resetRetry || prevUrl !== url) {
      retryLeftRef.current = optsRef.current?.errorCount ?? 5;
    }

    clearTimer();
    const oldWs = wsRef.current;
    closeWs(oldWs);

    const ws = new WebSocket(url);
    wsRef.current = ws;

    const scheduleReconnect = () => {
      if (manualCloseRef.current || unmountedRef.current) return;
      if (wsRef.current !== ws) return;
      if (optsRef.current?.shouldReconnect === false) return;
      if (!urlRef.current || timerRef.current) return;

      retryLeftRef.current -= 1;
      if (retryLeftRef.current < 0) {
        optsRef.current?.onError?.();
        return;
      }

      const interval = optsRef.current?.retryInterval ?? 2500;
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        if (manualCloseRef.current || unmountedRef.current) return;
        connect(urlRef.current, false);
      }, interval);
    };

    ws.onopen = () => {
      if (wsRef.current !== ws) return;
      retryLeftRef.current = optsRef.current?.errorCount ?? 5;
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

  useEffect(() => {
    return () => {
      unmountedRef.current = true;
      disconnect();
    };
  }, [disconnect]);

  return { connect, disconnect };
};
