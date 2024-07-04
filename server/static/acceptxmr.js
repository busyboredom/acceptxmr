// Try to load existing invoice on page load.
async function init() {
    const urlParams = new URLSearchParams(window.location.search);
    const id = urlParams.get('id');
    let response = await fetch("/invoice?id=" + id);
    if (response.status !== 410 && response.status !== 404 ) {
        let invoiceUpdate = await response.json();
        displayInvoiceUpdate(invoiceUpdate);
        newWebsocket();
    }
}
init()

function displayInvoiceUpdate(invoiceUpdate) {
    console.log(invoiceUpdate);

    // Show paid/due.
    document.getElementById("paid").innerHTML = picoToXMR(invoiceUpdate.amount_paid);
    document.getElementById("due").innerHTML = picoToXMR(invoiceUpdate.amount_requested);

    // Show instructive text depending on invoice state.
    var instructionString = "Loading...";
    var instructionClass = "";
    var addressCopyButtonDisabled = false;
    var closeReason = null;
    if (invoiceUpdate.amount_paid >= invoiceUpdate.amount_requested) {
        instructionString = "Paid! Thank you";
        closeReason = "Paid";
    } else if (invoiceUpdate.expiration_in > 2) {
        instructionString = "Send Monero to Address Below";
    } else if (invoiceUpdate.expiration_in > 0) {
        instructionString = "Address Expiring Soon";
        instructionClass += " warning";
        addressCopyButtonDisabled = true;
    } else {
        instructionString = "Address Expired!";
        closeReason = "Expired";
        addressCopyButtonDisabled = true;
    }
    document.getElementById("state-message").innerHTML = instructionString;
    document.getElementById("state-message").classList = instructionClass;

    // Hide address if nearing expiration.
    document.getElementById("address-copy-btn").disabled = addressCopyButtonDisabled;
    if (!addressCopyButtonDisabled) {
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
    const urlParams = new URLSearchParams(window.location.search);
    const id = urlParams.get('id');
    window.acceptxmrSocket = new WebSocket("ws://localhost:8080/invoice/ws?id=" + id);

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
