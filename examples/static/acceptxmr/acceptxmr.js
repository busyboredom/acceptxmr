async function start() {
    var message = document.getElementById("message").value;

    // Hide prep stuff, show payment stuff.
    document.getElementById("instruction").innerHTML = "Loading...";
    document.getElementById("preperation-content").style.display = "None";
    document.getElementById("payment-content").style.display = "inherit"

    const checkOutInfo = {
        method: "POST",
        body: JSON.stringify({
            "message": message
        }),
        headers: {
            'content-type': 'application/json'
        }
    }
    await fetch("/check_out", checkOutInfo);

    let socket = new WebSocket("ws://localhost:8080/ws/");

    socket.onmessage = function (event) {
        var invoiceUpdate = JSON.parse(event.data);

        // Show paid/due.
        document.getElementById("paid").innerHTML = picoToXMR(invoiceUpdate.amount_paid);
        document.getElementById("due").innerHTML = picoToXMR(invoiceUpdate.amount_requested);

        // Show confirmations/required.
        var confirmations = Math.max(0, invoiceUpdate.confirmations);
        document.getElementById("confirmations").innerHTML = confirmations;
        document.getElementById("confirmations-required").innerHTML = invoiceUpdate.confirmations_required;

        // Show instructive text depending on invoice state.
        var instructionString = "Loading...";
        var instructionClass = "acceptxmr-instruction";
        var newAddressBtnHidden = true;
        if (confirmations >= invoiceUpdate.confirmations_required) {
            instructionString = "Paid! Thank you"
            socket.close();
        } else if (invoiceUpdate.amount_paid > invoiceUpdate.amount_requested) {
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
            socket.close();
        }
        document.getElementById("instruction").innerHTML = instructionString;
        document.getElementById("instruction").classList = instructionClass;

        // Hide address if nearing expiration.
        document.getElementById("new-address-btn").hidden = newAddressBtnHidden;
        document.getElementById("address-copy-btn").disabled = !newAddressBtnHidden;
        if (newAddressBtnHidden) {
            var address = invoiceUpdate.address;
            document.getElementById("address").innerHTML = address;

            var qr = qrcode(0, "M");
            qr.addData(address);
            qr.make();
            document.getElementById('qrcode-container').innerHTML = qr.createSvgTag({ scalable: true });
        } else {
            document.getElementById("address").innerHTML = "Expiring or expired...";
            document.getElementById('qrcode-container').innerHTML = "<svg viewBox=\"0 0 100 100\" src=\"\"></svg>";
        }

    };

    // If the websocket closes cleanly, log it. Otherwise, alert the user.
    socket.onclose = function (event) {
        if (event.code === 1000) {
            console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
        } else {
            // Server process killed or network down.
            // Event.code is usually 1006 in this case.
            alert('Connection died. If you have paid already, rest assured that it will still be processed.');
        }
    };

    socket.onerror = function (error) {
        alert(`[error] ${error.message}`);
    };
}

// Convert from piconeros to monero.
function picoToXMR(amount) {
    let divisor = 1_000_000_000_000;
    let xmr = Math.floor(amount / divisor) + amount % divisor / divisor;
    return new Intl.NumberFormat(undefined, { maximumSignificantDigits: 20 }).format(xmr);
}

// Make the copy button work.
function copyInvoiceAddress() {
    // Get the text field
    var copyText = document.getElementById("address");

    // Copy the text inside the text field
    navigator.clipboard.writeText(copyText.innerHTML);

    // Provide feedback
    document.getElementById("address-copy-btn").innerHTML = "Copied!";
    setTimeout(function () {
        document.getElementById("address-copy-btn").innerHTML = "Copy";
    }, 1000);
}
