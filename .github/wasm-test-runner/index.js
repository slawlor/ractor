import puppeteer from "puppeteer";
import Xvfb from "xvfb";
import { spawn } from "child_process";
(async () => {
    const workingDir = process.env.WORKING_DIR;
    if (workingDir === undefined) {
        console.error("Set `WORKING_DIR` to the directory of ractor");
        return;
    }
    const cargoRunner = spawn("wasm-pack test --chrome ./ractor", { cwd: workingDir, stdio: "pipe", shell: true });
    cargoRunner.stdout.setEncoding("utf-8");
    cargoRunner.stderr.setEncoding("utf-8");
    const flagPromise = new Promise((resolve) => {
        cargoRunner.stdout.on("data", (data) => {
            console.log(data);
            if (data.includes("http://127.0.0.1:8000")) {
                console.log("flag captured");
                resolve();
            }
        });
    });
    cargoRunner.stderr.on("data", (data) => {
        process.stderr.write(data);
    });
    const xvfb = new Xvfb({
        silent: true,
        xvfb_args: ["-screen", "0", '1280x720x24', "-ac"],
    });
    await new Promise((resolve) => xvfb.start(resolve));
    const browser = await puppeteer.launch({
        headless: false,
        defaultViewport: null,
        args: ['--no-sandbox', '--start-fullscreen', '--display=' + xvfb.display()]
    });
    console.log(await browser.version());
    const page = await browser.newPage();
    await Promise.race([
        flagPromise,
        new Promise((_, rej) => setTimeout(() => {
            rej(new Error("Timed out when launching wasm-pack test"))
        }, 5 * 60 * 1000))
    ]);
    await page.goto(`http://127.0.0.1:8000`);
    await Promise.race([
        (async () => {
            let lastLog = null;
            while (true) {
                const logOutput = await page.$("#output");

                if (logOutput === null) {
                    console.log("#output not found, waiting..");
                } else {
                    const log = await logOutput.evaluate((hd) => hd.innerText);
                    let currentLogLine;
                    if (lastLog === null) {
                        currentLogLine = log
                    } else {
                        currentLogLine = log.slice(lastLog.length);
                    }
                    lastLog = log;
                    process.stdout.write(currentLogLine);
                    if (currentLogLine.includes("test result: FAILED")) {
                        throw new Error("Test failed");
                    } else if (currentLogLine.includes("test result: ok")) {
                        return;
                    }
                }

                await new Promise(res => setTimeout(res, 1000));
            }
        })(),
        new Promise((_, rej) => setTimeout(() => {
            rej(new Error("Timed out when running tests.."))
        }, 5 * 60 * 1000))
    ]);

    await page.close();
    await browser.close()
    await new Promise((resolve) => xvfb.stop(resolve));
    cargoRunner.kill();
    process.exit(0);
})()
