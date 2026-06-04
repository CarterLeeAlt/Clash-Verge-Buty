import axios from "axios";
import { cmdGetProxyDelay } from "./cmds";
import { getProxyDelay } from "./api";

const hashKey = (name: string, group: string) => `${group ?? ""}::${name}`;

type DelayTaskState = {
  id: number;
  aborted: boolean;
  controller: AbortController;
};

class DelayManager {
  private cache = new Map<string, [number, number]>();
  private urlMap = new Map<string, string>();
  private groupTaskMap = new Map<string, DelayTaskState>();
  private taskSeq = 0;

  // 每个item的监听
  private listenerMap = new Map<string, (time: number) => void>();

  // 每个分组的监听
  private groupListenerMap = new Map<string, () => void>();

  setUrl(group: string, url: string) {
    this.urlMap.set(group, url);
  }

  getUrl(group: string) {
    return this.urlMap.get(group);
  }

  setListener(name: string, group: string, listener: (time: number) => void) {
    const key = hashKey(name, group);
    this.listenerMap.set(key, listener);
  }

  removeListener(name: string, group: string) {
    const key = hashKey(name, group);
    this.listenerMap.delete(key);
  }

  setGroupListener(group: string, listener: () => void) {
    this.groupListenerMap.set(group, listener);
  }

  removeGroupListener(group: string) {
    this.groupListenerMap.delete(group);
  }

  cancelGroupCheck(group: string) {
    const task = this.groupTaskMap.get(group);
    if (task) {
      task.aborted = true;
      task.controller.abort();
    }
  }

  startGroupCheck(group: string) {
    this.cancelGroupCheck(group);
    const task = {
      id: ++this.taskSeq,
      aborted: false,
      controller: new AbortController(),
    };
    this.groupTaskMap.set(group, task);
    return task;
  }

  isCurrentGroupCheck(group: string, task: DelayTaskState) {
    return !task.aborted && this.groupTaskMap.get(group)?.id === task.id;
  }

  setDelay(name: string, group: string, delay: number) {
    const key = hashKey(name, group);
    this.cache.set(key, [Date.now(), delay]);
    this.listenerMap.get(key)?.(delay);
    this.groupListenerMap.get(group)?.();
  }

  getDelay(name: string, group: string) {
    if (!name) return -1;

    const result = this.cache.get(hashKey(name, group));
    if (result && Date.now() - result[0] <= 18e5) {
      return result[1];
    }
    return -1;
  }

  /// 暂时修复provider的节点延迟排序的问题
  getDelayFix(proxy: IProxyItem, group: string) {
    if (!proxy.provider) {
      const delay = this.getDelay(proxy.name, group);
      if (delay >= 0 || delay === -2) return delay;
    }

    if (proxy.history.length > 0) {
      // 0ms以error显示
      return proxy.history[proxy.history.length - 1].delay || 1e6;
    }
    return -1;
  }

  private isAbortError(err: any, signal?: AbortSignal) {
    return (
      signal?.aborted ||
      axios.isCancel(err) ||
      err?.name === "AbortError" ||
      err?.name === "CanceledError" ||
      err?.code === "ERR_CANCELED"
    );
  }

  private requestDelay(
    name: string,
    group: string,
    timeout: number
  ): Promise<number>;

  private requestDelay(
    name: string,
    group: string,
    timeout: number,
    signal: AbortSignal
  ): Promise<number | null>;

  private async requestDelay(
    name: string,
    group: string,
    timeout: number,
    signal?: AbortSignal
  ) {
    let delay = -1;

    try {
      const url = this.getUrl(group);
      const result = signal
        ? await getProxyDelay(name, { url, timeout, signal })
        : await cmdGetProxyDelay(name, timeout, url);
      delay = result.delay;
    } catch (err) {
      if (this.isAbortError(err, signal)) return null;
      delay = 1e6; // error
    }

    return delay;
  }

  async checkDelay(name: string, group: string, timeout: number) {
    const delay = await this.requestDelay(name, group, timeout);

    this.setDelay(name, group, delay);
    return delay;
  }

  async checkListDelay(
    nameList: string[],
    group: string,
    timeout: number,
    concurrency = 36,
    task = this.startGroupCheck(group)
  ) {
    const names = nameList.filter(Boolean);

    // 设置正在延迟测试中
    names.forEach((name) => {
      if (this.isCurrentGroupCheck(group, task)) {
        this.setDelay(name, group, -2);
      }
    });

    let nextIndex = 0;
    const workerCount = Math.min(concurrency, names.length);

    const help = async (): Promise<void> => {
      while (this.isCurrentGroupCheck(group, task)) {
        const name = names[nextIndex++];
        if (!name) return;

        const delay = await this.requestDelay(
          name,
          group,
          timeout,
          task.controller.signal
        );

        if (!this.isCurrentGroupCheck(group, task)) return;
        if (delay == null) return;
        this.setDelay(name, group, delay);
      }
    };

    await Promise.allSettled(Array.from({ length: workerCount }, () => help()));

    return this.isCurrentGroupCheck(group, task);
  }

  formatDelay(delay: number, timeout = 10000) {
    if (delay <= 0) return "Error";
    if (delay > 1e5) return "Error";
    if (delay >= timeout) return "Timeout"; // 10s
    return `${delay} ms`;
  }

  formatDelayColor(delay: number, timeout = 10000) {
    if (delay >= timeout) return "error.main";
    if (delay <= 0) return "error.main";
    if (delay > 500) return "warning.main";
    return "success.main";
  }
}

export default new DelayManager();
