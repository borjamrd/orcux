// Written by orcux. Tells it what this OpenCode does; its own subagents do not count.
import { spawn } from "node:child_process"

export const Orcux = async () => {
  if (!process.env.TMUX_PANE) return {}
  const children = new Set()
  let last = ""
  let ended = ""
  let queue = Promise.resolve()
  const tell = (state) => {
    if (state === last) return
    last = state
    // one after another: they must arrive in order
    queue = queue.then(() => new Promise((done) => {
      try {
        const p = spawn("ORCUX_EXE", ["hook", state, "exact"], { stdio: "ignore" })
        p.on("exit", done)
        p.on("error", done)
      } catch {
        done()
      }
    }))
  }
  return {
    event: async ({ event }) => {
      const p = event.properties ?? {}
      if (event.type === "session.created" && p.info?.parentID) children.add(p.info.id)
      const asks = /^(permission|question)\./.test(event.type)
      if (children.has(p.sessionID) && !asks) return
      if (event.type === "session.status" && p.status?.type !== "idle") {
        ended = ""
        tell("working")
      } else if (event.type === "session.error") {
        ended = p.error?.name === "MessageAbortedError" ? "idle" : "failed"
        tell(ended)
      } else if (event.type === "session.idle") {
        if (!ended) tell("done")
      } else if (event.type === "permission.asked") {
        tell("permission")
      } else if (event.type === "question.asked") {
        tell("question")
      } else if (asks) {
        tell("working") // replied or rejected
      }
    },
  }
}
