use std::str::FromStr;

use alloy::{
    network::{Ethereum, EthereumWallet, NetworkWallet, ReceiptResponse, TransactionBuilder},
    node_bindings::AnvilInstance,
    primitives::U256,
    providers::{ext::AnvilApi, Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    sol,
};
use anyhow::Result;

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract Counter {
        uint256 public number;
        function setNumber(uint256 newNumber) public {
            number = newNumber;
        }
        function increment() public {
            number++;
        }
    }
}

async fn deploy_and_interact(anvil: AnvilInstance, wallet: EthereumWallet) -> Result<()> {
    let provider = ProviderBuilder::new().wallet(wallet).on_http(
        format!("http://localhost:{}", anvil.port())
            .parse()
            .unwrap(),
    );

    // Deploy Counter contract
    let bytecode = hex::decode(
        "6080806040523460135760df908160198239f35b600080fdfe6080806040526004361015601257600080fd5b60003560e01c9081633fb5c1cb1460925781638381f58a146079575063d09de08a14603c57600080fd5b3460745760003660031901126074576000546000198114605e57600101600055005b634e487b7160e01b600052601160045260246000fd5b600080fd5b3460745760003660031901126074576020906000548152f35b34607457602036600319011260745760043560005500fea2646970667358221220e978270883b7baed10810c4079c941512e93a7ba1cd1108c781d4bc738d9090564736f6c634300081a0033"
    )?;
    let tx = TransactionRequest::default().with_deploy_code(bytecode);

    let receipt = provider
        .send_transaction(tx)
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let contract_address = receipt
        .contract_address()
        .expect("Failed to get contract address");
    let contract = Counter::new(contract_address, &provider);

    // Interact with contract
    contract
        .setNumber(U256::from(42))
        .send()
        .await?
        .watch()
        .await?;
    contract.increment().send().await?.watch().await?;
    let number = contract.number().call().await?.number;
    assert_eq!(number, U256::from(43));

    Ok(())
}

async fn deploy_and_interact_2(port: u16, wallet: EthereumWallet) -> Result<()> {
    let address = <EthereumWallet as NetworkWallet<Ethereum>>::default_signer_address(&wallet);

    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .on_http(format!("http://localhost:{port}").parse().unwrap());

    provider
        .anvil_set_balance(address, U256::from_str("1000000000000000000").unwrap())
        .await
        .unwrap();

    // Deploy Counter contract
    let bytecode = hex::decode(
        "6080806040523460135760df908160198239f35b600080fdfe6080806040526004361015601257600080fd5b60003560e01c9081633fb5c1cb1460925781638381f58a146079575063d09de08a14603c57600080fd5b3460745760003660031901126074576000546000198114605e57600101600055005b634e487b7160e01b600052601160045260246000fd5b600080fd5b3460745760003660031901126074576020906000548152f35b34607457602036600319011260745760043560005500fea2646970667358221220e978270883b7baed10810c4079c941512e93a7ba1cd1108c781d4bc738d9090564736f6c634300081a0033"
    )?;
    let tx = TransactionRequest::default().with_deploy_code(bytecode);

    let receipt = provider
        .send_transaction(tx)
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let contract_address = receipt
        .contract_address()
        .expect("Failed to get contract address");
    let contract = Counter::new(contract_address, &provider);

    // Interact with contract
    contract
        .setNumber(U256::from(42))
        .send()
        .await?
        .watch()
        .await?;
    contract.increment().send().await?.watch().await?;
    let number = contract.number().call().await?.number;
    assert_eq!(number, U256::from(43));

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use alloy::primitives::Address;
    use alloy::signers::local::PrivateKeySigner;
    use futures::future::join_all;
    use futures::Future;
    use once_cell::sync::Lazy;
    use parking_lot::RwLock;
    use std::collections::HashSet;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::Duration;

    // #[tokio::test]
    // async fn test_parallel_anvil() -> Result<()> {
    //     let mut handles = Vec::new();
    //     for i in 0..100 {
    //         let anvil = Anvil::new().block_time(1).try_spawn().unwrap();
    //
    //         println!("spawning anvil {i:?}");
    //
    //         let signer: PrivateKeySigner =
    //             "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    //                 .parse()
    //                 .unwrap();
    //         let wallet = EthereumWallet::from(signer);
    //
    //         handles.push(deploy_and_interact(anvil, wallet));
    //     }
    //
    //     let results = join_all(handles).await;
    //     for result in results {
    //         assert!(result.is_ok());
    //     }
    //
    //     Ok(())
    // }

    static USED_ADDRESSES: Lazy<Mutex<HashSet<Address>>> = Lazy::new(|| Mutex::new(HashSet::new()));

    fn generate_unique_wallet() -> EthereumWallet {
        loop {
            let signer = PrivateKeySigner::random();
            let wallet = EthereumWallet::from(signer);
            let address =
                <EthereumWallet as NetworkWallet<Ethereum>>::default_signer_address(&wallet);
            let mut addresses = USED_ADDRESSES.lock().unwrap();
            if !addresses.contains(&address) {
                addresses.insert(address);
                return wallet;
            }
        }
    }

    async fn get_anvil_port<F, Fut>(fun: F)
    where
        F: FnOnce(u16) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        static CONCURRENT_CALLS: Lazy<RwLock<usize>> = Lazy::new(|| RwLock::new(0));

        loop {
            if *CONCURRENT_CALLS.read() < 20 {
                break;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        {
            let mut calls = CONCURRENT_CALLS.write();
            *calls += 1;
        }

        let port = 8545;

        let _ = fun(port).await;

        // Decrement counter
        let mut calls = CONCURRENT_CALLS.write();
        *calls -= 1;
    }

    async fn test_parallel_single_anvil(port: u16) -> Result<()> {
        let wallet = generate_unique_wallet();

        deploy_and_interact_2(port, wallet).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_single_anvil() -> Result<()> {
        // Basic background process
        let mut child = Command::new("anvil")
            .spawn()
            .expect("Failed to start process");

        let mut handles = Vec::new();
        for i in 0..1000 {
            println!("{i}");
            let handle =
                tokio::spawn(async move { get_anvil_port(test_parallel_single_anvil).await });
            handles.push(handle);
        }
        let results = join_all(handles).await;
        for result in results {
            assert!(result.is_ok());
        }

        child.kill().unwrap();
        Ok(())
    }
}
