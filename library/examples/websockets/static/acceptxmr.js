// Try to load existing invoice on page load.
async function init() {
    let response = await fetch("/update");
    if (response.status !== 410 ) {
        let invoiceUpdate = await response.json();
        displayInvoiceUpdate(invoiceUpdate);
        await next(true);
    }
}
init()

async function next(hasAddress) {
    // Hide prep stuff, show payment stuff.
    document.getElementById("preparation-content").style.display = "None";
    document.getElementById("payment-content").style.display = "inherit";

    // Create invoice.
    if (!hasAddress) {
        document.getElementById("instruction").innerHTML = "Loading...";
        await newAddress();
    } else {
        newWebsocket();
    }
}

async function newAddress() {
    const message = document.getElementById("message").value;
    const checkOutInfo = {
        method: "POST",
        body: JSON.stringify({
            "message": message
        }),
        headers: {
            'content-type': 'application/json'
        }
    };

    await fetch("/checkout", checkOutInfo);
    newWebsocket();

    let response = await fetch("/update");
    let invoiceUpdate = await response.json();
    displayInvoiceUpdate(invoiceUpdate);
}

function displayInvoiceUpdate(invoiceUpdate) {
    console.log(invoiceUpdate);

    // Show paid/due.
    document.getElementById("paid").innerHTML = picoToXMR(invoiceUpdate.amount_paid);
    document.getElementById("due").innerHTML = picoToXMR(invoiceUpdate.amount_requested);

    // Show confirmations/required.
    var confirmations = invoiceUpdate.confirmations;
    document.getElementById("confirmations").innerHTML = Math.max(0, confirmations);
    document.getElementById("confirmations-required").innerHTML = invoiceUpdate.confirmations_required;

    // Show instructive text depending on invoice state.
    var instructionString = "Loading...";
    var instructionClass = "acceptxmr-instruction";
    var newAddressBtnHidden = true;
    var closeReason = null;
    if (confirmations !== null && confirmations >= invoiceUpdate.confirmations_required) {
        instructionString = "Paid! Thank you"
        closeReason = "Confirmed";
    } else if (invoiceUpdate.amount_paid >= invoiceUpdate.amount_requested) {
        instructionString = "Paid! Waiting for Confirmation..."
    } else if (invoiceUpdate.expiration_in > 2) {
        instructionString = "Send Monero to Address Below"
    } else if (invoiceUpdate.expiration_in > 0) {
        instructionString = "Address Expiring Soon";
        instructionClass += " warning";
        newAddressBtnHidden = false;
    } else {
        instructionString = "Address Expired!";
        newAddressBtnHidden = false;
        closeReason = "Expired";
    }
    document.getElementById("instruction").innerHTML = instructionString;
    document.getElementById("instruction").classList = instructionClass;

    // Hide address if nearing expiration.
    document.getElementById("new-address-btn").hidden = newAddressBtnHidden;
    document.getElementById("address-copy-btn").disabled = !newAddressBtnHidden;
    if (newAddressBtnHidden) {
        document.getElementById("address").innerHTML = invoiceUpdate.address;

        const qr = qrcode(0, "M");
        qr.addData(invoiceUpdate.uri);
        qr.make();
        document.getElementById('qrcode-container').innerHTML = qr.createSvgTag({ scalable: true });
    } else {
        document.getElementById("address").innerHTML = "Expiring or expired...";
        document.getElementById('qrcode-container').innerHTML = "<svg viewBox=\"0 0 100 100\" src=\"\"></svg>";
    }

    return closeReason;
}

function newWebsocket() {
    // Close websocket if it already exists.
    if (typeof window.acceptxmrSocket != 'undefined') {
        window.acceptxmrSocket.close(1000, "New Address");
    }

    // Open websocket.
    window.acceptxmrSocket = new WebSocket("ws://localhost:8080/ws/");

    window.acceptxmrSocket.onmessage = function (event) {
        let closeReason = displayInvoiceUpdate(JSON.parse(event.data));
        if (closeReason != null) {
            window.acceptxmrSocket.close(1000, closeReason);
        }
    }

    // If the websocket closes cleanly, log it. Otherwise, alert the user.
    window.acceptxmrSocket.onclose = function (event) {
        if (event.code === 1000) {
            console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
        } else {
            // Server process killed or network down.
            // Event.code is usually 1006 in this case.
            alert('Connection died. If you have paid already, rest assured that it will still be processed.');
        }
    };

    window.acceptxmrSocket.onerror = function (error) {
        alert(`[error] ${error.message}`);
    };
}

// Convert from piconeros to monero.
function picoToXMR(amount) {
    const divisor = 1_000_000_000_000;
    const xmr = Math.floor(amount / divisor) + amount % divisor / divisor;
    return new Intl.NumberFormat(undefined, { maximumSignificantDigits: 20 }).format(xmr);
}

// Make the copy button work.
function copyInvoiceAddress() {
    // Get the text field
    const copyText = document.getElementById("address");

    // Copy the text inside the text field
    navigator.clipboard.writeText(copyText.innerHTML);

    // Provide feedback
    document.getElementById("address-copy-btn").innerHTML = "Copied!";
    setTimeout(function () {
        document.getElementById("address-copy-btn").innerHTML = "Copy";
    }, 1000);
}
