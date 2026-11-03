"""Run from the repository root: python backend/tests/rpc_e2e.py.

Requires cargo, Foundry (forge/cast/anvil), PostgreSQL binaries, and Python 3.
Creates disposable PostgreSQL and Anvil instances on random localhost ports.
Uses only Anvil's public test key; never reads the configured production database.
"""

import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
os.chdir(ROOT)
urllib.request.install_opener(
    urllib.request.build_opener(urllib.request.ProxyHandler({}))
)
KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ZERO = "0x" + "00" * 20
SIGNER = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


@contextlib.contextmanager
def service(args, log_path, env):
    with open(log_path, "w+") as log:
        process = subprocess.Popen(args, stdout=log, stderr=log, env=env)
        try:
            yield process
        except BaseException:
            log.seek(0)
            print(log.read())
            raise
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def ready(check, process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("test service exited during startup")
        try:
            if check():
                return
        except (OSError, subprocess.CalledProcessError):
            pass
        time.sleep(0.1)
    raise RuntimeError("test service startup timed out")


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def send(to, sig, *args, value=None):
    cmd = [
        "cast",
        "send",
        "--rpc-url",
        RPC,
        "--private-key",
        KEY,
        "--json",
        to,
        sig,
        *map(str, args),
    ]
    if value:
        cmd.extend(["--value", str(value)])
    r = json.loads(command(*cmd))
    assert r["status"] == "0x1", r
    return r


def deploy(artifact, *args):
    data = json.load(open(artifact))["bytecode"]["object"]
    if args:
        data += command("cast", "abi-encode", *args)[2:]
    r = json.loads(
        command(
            "cast",
            "send",
            "--rpc-url",
            RPC,
            "--private-key",
            KEY,
            "--json",
            "--create",
            data,
        )
    )
    assert r["status"] == "0x1", r
    return r["contractAddress"]


def sql(query):
    return command("psql", DATABASE_URL, "-At", "-v", "ON_ERROR_STOP=1", "-c", query)


def rpc(method, params):
    req = urllib.request.Request(
        API_URL + "/rpc",
        data=json.dumps(
            dict(jsonrpc="2.0", id=42, method=method, params=params)
        ).encode(),
        headers={"Content-Type": "application/json"},
    )
    r = json.load(urllib.request.urlopen(req))
    assert r["id"] == 42
    return r


def exercise():
    manager = deploy(
        "out/PoolManager.sol/PoolManager.json", "constructor(address)", SIGNER
    )
    token = deploy(
        "out/mocks/MockERC20.sol/MockERC20.json",
        "constructor(string,string,uint8)",
        "USDT",
        "USDT",
        "6",
    )
    router = deploy(
        "out/PoolModifyLiquidityTest.sol/PoolModifyLiquidityTest.json",
        "constructor(address)",
        manager,
    )
    key = f"({ZERO},{token},3000,60,{ZERO})"
    pool_id = command(
        "cast",
        "keccak",
        command(
            "cast",
            "abi-encode",
            "f(address,address,uint24,int24,address)",
            ZERO,
            token,
            "3000",
            "60",
            ZERO,
        ),
    )
    send(token, "mint(address,uint256)", SIGNER, 10**25)
    send(token, "approve(address,uint256)", router, 2**256 - 1)
    send(
        manager,
        "initialize((address,address,uint24,int24,address),uint160)",
        key,
        2**96,
    )
    send(
        router,
        "modifyLiquidity((address,address,uint24,int24,address),(int24,int24,int256,bytes32),bytes)",
        key,
        f"(-120,120,{10**18},0x" + ("00" * 32) + ")",
        "0x",
        value=10**18,
    )
    sql(
        f"INSERT INTO ethereum_config (rpc_url,chain_id,uniswapv4_core_pool_manager) VALUES ('{RPC}',31337,decode('{manager[2:]}','hex'))"
    )
    sql(
        f"INSERT INTO pools(protocol,chain_id,pool_id,base_contract,token0,token1,fee,tick_spacing,hooks) VALUES ('uni_v4',31337,decode('{pool_id[2:]}','hex'),decode('{manager[2:]}','hex'),decode('{ZERO[2:]}','hex'),decode('{token[2:]}','hex'),3000,60,decode('{ZERO[2:]}','hex'))"
    )
    r = rpc("deploy_swapper", dict(chain_id=31337))
    assert "result" in r, r
    swapper = r["result"]["contract_address"]
    assert (
        sql("SELECT encode(interaction_contract,'hex') FROM pools WHERE chain_id=31337")
        == swapper[2:].lower()
    )
    assert (
        sql(
            "SELECT encode(my_swapper_contract,'hex') FROM ethereum_config WHERE chain_id=31337"
        )
        == swapper[2:].lower()
    )
    for inp, out in [(ZERO, token), (token, ZERO)]:
        r = rpc(
            "swap",
            dict(
                chain_id=31337,
                pool_id=pool_id,
                input_token=inp,
                output_token=out,
                input_amount=str(10**12),
            ),
        )
        assert "result" in r, r
        result = r["result"]
        assert int(result["output_amount"]) > 0, result
        assert result["slippage"] == "0.000000000000000000", result
        assert len(result["tx_id"]) == 66, result
        print("PASS:", "native input" if inp == ZERO else "ERC20 input")
    # Wrong token pairs and unknown IDs fail; multiple pools for a pair are selectable.
    r = rpc(
        "swap",
        dict(
            chain_id=31337,
            pool_id=pool_id,
            input_token=ZERO,
            output_token=SIGNER,
            input_amount="100",
        ),
    )
    assert r["error"]["code"] == -32000, r
    sql(
        "INSERT INTO pools(protocol,chain_id,pool_id,base_contract,interaction_contract,token0,token1,fee,tick_spacing,hooks) SELECT protocol,chain_id,decode(repeat('ab',32),'hex'),base_contract,interaction_contract,token0,token1,500,10,hooks FROM pools WHERE chain_id=31337"
    )
    r = rpc(
        "swap",
        dict(
            chain_id=31337,
            pool_id=pool_id,
            input_token=ZERO,
            output_token=token,
            input_amount="1000000000000",
        ),
    )
    assert "result" in r, r
    r = rpc(
        "swap",
        dict(
            chain_id=31337,
            pool_id="0x" + "ff" * 32,
            input_token=ZERO,
            output_token=token,
            input_amount="100",
        ),
    )
    assert r["error"]["code"] == -32000, r
    print("DEPLOYMENT PERSISTENCE AND ROUTE FAILURE CHECKS PASS")


if __name__ == "__main__":
    subprocess.run(
        ["forge", "build", "--skip", "test", "--skip", "script", "--silent"], check=True
    )
    subprocess.run(
        [
            "cargo",
            "build",
            "--manifest-path",
            "backend/Cargo.toml",
            "--bin",
            "backend",
            "--bin",
            "db",
        ],
        check=True,
    )
    with tempfile.TemporaryDirectory(prefix="v4-rpc-e2e-") as temporary:
        pg_port, rpc_port, api_port = free_port(), free_port(), free_port()
        DATABASE_URL = f"postgresql://postgres@127.0.0.1:{pg_port}/postgres"
        RPC = f"http://127.0.0.1:{rpc_port}"
        API_URL = f"http://127.0.0.1:{api_port}"
        env = dict(
            os.environ,
            DATABASE_URL=DATABASE_URL,
            RPC_URL=RPC,
            PRIVATE_KEY=KEY,
            API_ADDR=f"127.0.0.1:{api_port}",
            NO_PROXY="127.0.0.1,localhost",
            no_proxy="127.0.0.1,localhost",
            RUST_LOG="warn",
        )
        data = temporary + "/postgres"
        subprocess.run(
            ["initdb", "-D", data, "-A", "trust", "-U", "postgres"],
            check=True,
            stdout=subprocess.DEVNULL,
        )

        def db(action):
            subprocess.run(
                ["make", "--no-print-directory", "-C", "backend", action],
                check=True,
                env=env,
                stdout=subprocess.DEVNULL,
            )

        with service(
            [
                "postgres",
                "-D",
                data,
                "-h",
                "127.0.0.1",
                "-p",
                str(pg_port),
                "-k",
                temporary,
            ],
            temporary + "/postgres.log",
            env,
        ) as pg:
            ready(lambda: sql("SELECT 1") == "1", pg)
            # Prove upgrade of an existing fixture row, before SQLx creates fresh tables.
            upgrade = (
                "BEGIN;"
                + "\n".join(
                    (ROOT / "backend/sql/create" / name).read_text()
                    for name in [
                        "001_create_ethereum_config.sql",
                        "002_create_pools.sql",
                    ]
                )
                + """
                INSERT INTO pools(protocol, chain_id, base_contract, token0, token1, fee, tick_spacing, hooks)
                VALUES ('uni_v4', 1, decode('000000000004444c5dc75cb358380d2e3de08a90', 'hex'),
                    decode(repeat('00',20), 'hex'), decode('dac17f958d2ee523a2206206994597c13d831ec7', 'hex'),
                    3000, 60, decode(repeat('00',20), 'hex'));
            """
                + (ROOT / "backend/sql/create/003_add_pool_id.sql").read_text()
                + "SELECT encode(pool_id, 'hex') FROM pools; ROLLBACK;"
            )
            assert (
                "50b00c1a5a8e582ec808e97e71598cd135206a9f9c548eab2ed73659e7ee5fa8"
                in sql(upgrade)
            )
            db("db-migrate")
            db("db-migrate")
            db("db-load")
            assert (
                sql(
                    "SELECT count(*) FROM pools WHERE chain_id=1 AND interaction_contract IS NULL"
                )
                == "1"
            )
            db("db-purify")
            assert sql("SELECT count(*) FROM pools") == "0"
            assert sql("SELECT count(*) FROM ethereum_config") == "0"
            db("db-load")
            db("db-reload")
            assert sql("SELECT count(*) FROM ethereum_config") == "1"
            assert sql("SELECT count(*) FROM pools") == "1"
            assert (
                sql("SELECT encode(pool_id, 'hex') FROM pools")
                == "50b00c1a5a8e582ec808e97e71598cd135206a9f9c548eab2ed73659e7ee5fa8"
            )
            with service(
                ["anvil", "--port", str(rpc_port), "--chain-id", "31337", "--silent"],
                temporary + "/anvil.log",
                env,
            ) as anvil:
                ready(
                    lambda: command("cast", "chain-id", "--rpc-url", RPC) == "31337",
                    anvil,
                )
                with service(
                    ["backend/target/debug/backend"], temporary + "/api.log", env
                ) as api:
                    ready(
                        lambda: rpc("app_unknown", {})["error"]["code"] == -32601, api
                    )
                    for path in ["/hello/test", "/chains/31337/swapper"]:
                        try:
                            urllib.request.urlopen(API_URL + path, timeout=1)
                        except urllib.error.HTTPError as error:
                            assert error.code == 405, error
                        else:
                            raise AssertionError(
                                f"REST endpoint still responds: {path}"
                            )
                    legacy_deploy = urllib.request.Request(
                        API_URL + "/chains/31337/swapper",
                        data=b"{}",
                        headers={"Content-Type": "application/json"},
                    )
                    with urllib.request.urlopen(legacy_deploy, timeout=1) as response:
                        assert response.status == 200
                        error = json.load(response)
                        assert error["error"]["code"] in (-32600, -32700), error
                    exercise()
                assert api.returncode == 0, "JSON-RPC server did not shut down cleanly"
            db("db-clear-structs")
            assert (
                sql(
                    "SELECT to_regclass('pools') IS NULL AND to_regclass('ethereum_config') IS NULL"
                )
                == "t"
            )
            db("db-migrate")
            print("PASS: fixtures, purify, revert, and migration reapplication")
